// SPDX-License-Identifier: AGPL-3.0-only
//! Main-thread bootstrap (`wasm32` only): spawn the data worker, forward
//! configuration and visibility, bind DOM click/key/input/pointer/hash/wheel
//! events across all 8 Areas and 27 Sub-Views, handle WebAuthn Passkey
//! registration & `Timed` role elevation ceremonies, and synchronize browser
//! storage and hash routes across both [`HtmlCoreSkin`] and [`WireframeSkin`].

use std::cell::RefCell;
use std::rc::Rc;

use js_sys::{Array, ArrayBuffer, Date, Function, Object, Promise, Reflect, Uint8Array, JSON};
use mu_console_layout::{SafeArea, Viewport};
use mu_console_protocol::{dto::paths, AuthStatus, UiCommand, WorkerConfig};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{
    console, Element, Event, HtmlElement, HtmlInputElement, KeyboardEvent, MessageEvent,
    MouseEvent, WheelEvent, Worker,
};

use crate::client::{reload_banner_text, ClientUpdate, DataClient, WORKER_URL};
use crate::components::{filter_placeholder, render_console, render_html_shell};
use crate::copy;
use crate::router::{format_hash, has_explicit_route};
use crate::state::{
    extract_invite_token, font_slug, is_action_root_boundary, is_passive_container_control_id,
    skin_slug, ConsoleState, WebAuthnRequest, ACTION_ATTRS, FONT_STORAGE_KEY,
    PENDING_HASH_STORAGE_KEY, PENDING_INVITE_STORAGE_KEY, REPO_STORAGE_KEY, SKIN_STORAGE_KEY,
};
use crate::webauthn::{
    b64url_decode, b64url_encode, elevate_start_body, passkey_manage_start_body,
    register_start_body, wrap_credential_envelope_json,
};

thread_local! {
    static WORKER: RefCell<Option<Worker>> = const { RefCell::new(None) };
    static LAST_RENDER: RefCell<String> = const { RefCell::new(String::new()) };
    static LAST_SHELL_AUTH: RefCell<Option<(AuthStatus, bool, bool)>> = const { RefCell::new(None) };
}

fn now_ms() -> u64 {
    Date::now() as u64
}

/// Post a command as a transferred `ArrayBuffer`.
fn send(worker: &Worker, cmd: &UiCommand) {
    if let Ok(bytes) = DataClient::encode_command(cmd) {
        let buf = Uint8Array::from(bytes.as_slice()).buffer();
        let _ = worker.post_message_with_transfer(&buf, &Array::of1(&buf));
    }
}

fn send_all(worker: &Worker, cmds: &[UiCommand]) {
    for cmd in cmds {
        send(worker, cmd);
    }
}

fn sync_browser_state(st: &mut ConsoleState) {
    let Some(win) = web_sys::window() else {
        return;
    };
    if let Some(doc_el) = win.document().and_then(|d| d.document_element()) {
        let _ = doc_el.set_attribute("lang", "en");
        let _ = doc_el.set_attribute("data-skin", skin_slug(st.effective_skin()));
    }

    let desired_hash = format_hash(&st.route);
    let current_hash = win.location().hash().unwrap_or_default();
    if st.route_sync_dirty || current_hash != desired_hash {
        let _ = win.location().set_hash(&desired_hash);
        st.route_sync_dirty = false;
    }

    if let Ok(Some(storage)) = win.local_storage() {
        let _ = storage.set_item(FONT_STORAGE_KEY, font_slug(st.font));
        let _ = storage.set_item(SKIN_STORAGE_KEY, skin_slug(st.skin));
        if let Some(repo_str) = st.stored_repo_value() {
            let _ = storage.set_item(REPO_STORAGE_KEY, &repo_str);
        } else {
            let _ = storage.remove_item(REPO_STORAGE_KEY);
        }
    }

    if let Ok(Some(session)) = win.session_storage() {
        let auth = st.auth();
        if let Some(ref deep) = st.stashed_deep_link {
            let _ = session.set_item(PENDING_HASH_STORAGE_KEY, deep);
        } else if !matches!(
            auth.status,
            AuthStatus::Active | AuthStatus::PasskeyEnrollmentRequired
        ) && st.has_explicit_hash
            && !st.can_access_area(st.route.area)
        {
            let _ = session.set_item(PENDING_HASH_STORAGE_KEY, &desired_hash);
        } else if auth.status == AuthStatus::Active {
            let _ = session.remove_item(PENDING_HASH_STORAGE_KEY);
        }
    }
}

fn show_banner(text: &str) {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let el = doc.get_element_by_id("muconsole-reload").or_else(|| {
        let el = doc.create_element("div").ok()?;
        el.set_id("muconsole-reload");
        let _ = el.set_attribute("role", "alert");
        let _ = el.set_attribute(
            "style",
            "position:fixed;inset:0 0 auto 0;padding:12px 16px;font:16px system-ui;\
             background:#ffb43a;color:#060402;z-index:10",
        );
        doc.body()?.append_child(&el).ok()?;
        Some(el)
    });
    if let Some(el) = el {
        el.set_text_content(Some(text));
    }
}

fn ensure_dyslexic_stylesheet(doc: &web_sys::Document) {
    if doc.get_element_by_id("muconsole-dyslexic-font").is_some() {
        return;
    }
    if let Ok(style) = doc.create_element("style") {
        style.set_id("muconsole-dyslexic-font");
        style.set_text_content(Some(
            "@font-face{font-family:'OpenDyslexicM Nerd Font Mono';src:url('/fonts/OpenDyslexicMNerdFontMono-Regular.woff2') format('woff2');font-display:swap;}",
        ));
        if let Some(body) = doc.body() {
            let _ = body.append_child(&style);
        }
    }
}

fn window_viewport(win: &web_sys::Window) -> Viewport {
    let w = win
        .inner_width()
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(1512.0);
    let h = win
        .inner_height()
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(860.0);
    Viewport::new(w, h)
}

fn auth_status_slug(status: AuthStatus) -> &'static str {
    match status {
        AuthStatus::Unresolved => "unresolved",
        AuthStatus::Anonymous => "anonymous",
        AuthStatus::AuthorizationDenied => "authorization_denied",
        AuthStatus::PasskeyEnrollmentRequired => "passkey_required",
        AuthStatus::Active => "active",
    }
}

fn capture_focus(doc: &web_sys::Document) -> Option<(String, Option<u32>)> {
    let active = doc.active_element()?;
    let id = active.id();
    if id.is_empty() {
        return None;
    }
    let caret = active
        .dyn_ref::<HtmlInputElement>()
        .and_then(|inp| inp.selection_start().ok().flatten());
    Some((id, caret))
}

fn restore_focus(doc: &web_sys::Document, saved: Option<(String, Option<u32>)>) {
    let Some((id, caret)) = saved else {
        return;
    };
    let Some(el) = doc.get_element_by_id(&id) else {
        return;
    };
    if let Ok(html) = el.clone().dyn_into::<HtmlElement>() {
        let _ = html.focus();
    }
    if let Some(pos) = caret {
        if let Ok(inp) = el.dyn_into::<HtmlInputElement>() {
            let _ = inp.set_selection_range(pos, pos);
        }
    }
}

fn repaint(st: &ConsoleState) {
    let Some(win) = web_sys::window() else {
        return;
    };
    let Some(doc) = win.document() else {
        return;
    };

    let auth = st.auth();
    let eff_skin = st.effective_skin();
    if let Some(doc_el) = doc.document_element() {
        let _ = doc_el.set_attribute("data-skin", skin_slug(eff_skin));
    }

    let vp = window_viewport(&win);
    let rendered = render_console(st, vp, SafeArea::ZERO);
    let shell_key = (
        auth.status,
        auth.passkey_enrollment_required,
        st.session_probe_error().is_some(),
    );
    let shell_auth_changed = LAST_SHELL_AUTH.with(|prev| {
        let mut slot = prev.borrow_mut();
        if *slot == Some(shell_key) {
            false
        } else {
            *slot = Some(shell_key);
            true
        }
    });

    let focus_snapshot = capture_focus(&doc);
    let dash = doc.get_element_by_id("muconsole-dashboard");

    if let (Some(dash), false) = (dash, shell_auth_changed) {
        let content_changed = LAST_RENDER.with(|last| {
            let mut prev = last.borrow_mut();
            if *prev == rendered {
                false
            } else {
                prev.clone_from(&rendered);
                true
            }
        });
        if content_changed {
            dash.set_inner_html(&rendered);
            restore_focus(&doc, focus_snapshot);
        }

        if let Some(shell) = doc.get_element_by_id("muconsole-shell") {
            let auth_slug = auth_status_slug(auth.status);
            let _ = shell.set_attribute("data-crt-font", font_slug(st.font));
            let _ = shell.set_attribute("data-skin", skin_slug(eff_skin));
            let _ = shell.set_attribute(
                "data-lazy-dyslexic",
                if st.lazy_dyslexic_loaded {
                    "true"
                } else {
                    "false"
                },
            );
            let _ = shell.set_attribute("data-area", st.route.area.slug());
            let _ = shell.set_attribute("data-subview", st.route.subview.slug());
            let _ = shell.set_attribute("data-auth-state", auth_slug);

            if let Some(roles_btn) = doc.get_element_by_id("muconsole-roles-btn") {
                roles_btn.set_text_content(Some(&st.format_header_roles()));
            }
            if let Some(auth_link) = doc.get_element_by_id("muconsole-auth-link") {
                let (href, label) = match auth.status {
                    AuthStatus::Anonymous => (st.sign_in_href(), copy::SIGN_IN_LABEL),
                    AuthStatus::AuthorizationDenied
                    | AuthStatus::PasskeyEnrollmentRequired
                    | AuthStatus::Active => (st.sign_out_href(), copy::SIGN_OUT_LABEL),
                    AuthStatus::Unresolved => {
                        if st.session_probe_error().is_some() {
                            (st.sign_out_href(), copy::SIGN_OUT_LABEL)
                        } else {
                            ("#".to_owned(), copy::LOADING_SESSION_TITLE)
                        }
                    }
                };
                let _ = auth_link.set_attribute("href", &href);
                let _ = auth_link.set_attribute("data-auth-state", auth_slug);
                auth_link.set_text_content(Some(label));
            }
        }
        if st.lazy_dyslexic_loaded {
            ensure_dyslexic_stylesheet(&doc);
        }
        if let Some(polite) = doc.get_element_by_id("muconsole-aria-polite") {
            polite.set_text_content(Some(&st.aria_polite));
        }
        if let Some(assertive) = doc.get_element_by_id("muconsole-aria-assertive") {
            assertive.set_text_content(Some(&st.aria_assertive));
        }
        if let Some(input) = doc
            .get_element_by_id("muconsole-filter")
            .and_then(|e| e.dyn_into::<HtmlInputElement>().ok())
        {
            let _ = input.set_attribute("placeholder", filter_placeholder(st.route.area));
            if input.value() != st.filter_query {
                input.set_value(&st.filter_query);
            }
        }
    } else if let Some(body) = doc.body() {
        LAST_RENDER.with(|last| *last.borrow_mut() = rendered.clone());
        let html = render_html_shell(&rendered, st);
        body.set_inner_html(&html);
        if st.lazy_dyslexic_loaded {
            ensure_dyslexic_stylesheet(&doc);
        }
        restore_focus(&doc, focus_snapshot);
    }
}

fn find_action_attribute(target: &Element) -> Option<(&'static str, String)> {
    let mut cur = Some(target.clone());
    while let Some(el) = cur {
        let el_id = el.id();
        if is_action_root_boundary(&el_id) {
            break;
        }
        if let Some(val) = el.get_attribute("data-control-id") {
            if val.ends_with(":prev") || val.ends_with(":next") {
                return Some(("data-control-id", val));
            }
        }
        for attr in ACTION_ATTRS {
            if let Some(val) = el.get_attribute(attr) {
                return Some((attr, val));
            }
        }
        if let Some(val) = el.get_attribute("data-control-id") {
            if !is_passive_container_control_id(&val) {
                return Some(("data-control-id", val));
            }
        }
        cur = el.parent_element();
    }
    None
}

fn handle_post_action_effects(
    st: &mut ConsoleState,
    cmds: &[UiCommand],
) -> Option<WebAuthnRequest> {
    if let Some(ext) = st.pending_external_url.take() {
        if let Some(win) = web_sys::window() {
            let _ =
                win.open_with_url_and_target_and_features(&ext, "_blank", "noopener,noreferrer");
        }
    }

    for cmd in cmds {
        if let UiCommand::Navigate { ref href } = cmd {
            if let Some(win) = web_sys::window() {
                if href.starts_with("/oauth2/") {
                    if let Ok(Some(session)) = win.session_storage() {
                        let hash = st
                            .stashed_deep_link
                            .clone()
                            .unwrap_or_else(|| format_hash(&st.route));
                        if has_explicit_route(&hash) {
                            let _ = session.set_item(PENDING_HASH_STORAGE_KEY, &hash);
                        }
                    }
                }
                let _ = win.location().set_href(href);
            }
        }
    }

    st.pending_webauthn.take()
}

fn b64url_to_uint8_array(s: &str) -> Uint8Array {
    let bytes = b64url_decode(s);
    Uint8Array::from(bytes.as_slice())
}

fn js_buffer_prop_to_b64url(obj: &JsValue, prop: &str) -> Option<String> {
    let val = Reflect::get(obj, &JsValue::from_str(prop)).ok()?;
    if val.is_undefined() || val.is_null() {
        return None;
    }
    let arr = Uint8Array::new(&val);
    let bytes = arr.to_vec();
    Some(b64url_encode(&bytes))
}

fn decode_credential_descriptor_ids(pk: &JsValue, list_key: &str) {
    let Ok(list_val) = Reflect::get(pk, &JsValue::from_str(list_key)) else {
        return;
    };
    if list_val.is_null() {
        if let Ok(pk_obj) = pk.clone().dyn_into::<Object>() {
            let _ = Reflect::delete_property(&pk_obj, &JsValue::from_str(list_key));
        }
        return;
    }
    let Some(arr) = list_val.dyn_ref::<Array>() else {
        return;
    };
    for i in 0..arr.length() {
        let item = arr.get(i);
        if let Some(id_str) = Reflect::get(&item, &JsValue::from_str("id"))
            .ok()
            .and_then(|v| v.as_string())
        {
            let _ = Reflect::set(
                &item,
                &JsValue::from_str("id"),
                &b64url_to_uint8_array(&id_str),
            );
        }
    }
}

fn extract_challenge_id(raw: &JsValue) -> Option<String> {
    Reflect::get(raw, &JsValue::from_str("challenge_id"))
        .ok()
        .and_then(|v| v.as_string())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

fn unwrap_public_key_options(raw: &JsValue) -> JsValue {
    if let Ok(opts) = Reflect::get(raw, &JsValue::from_str("options")) {
        if opts.is_object() {
            if let Ok(pk) = Reflect::get(&opts, &JsValue::from_str("publicKey")) {
                if pk.is_object() {
                    return pk;
                }
            }
            return opts;
        }
    }
    Reflect::get(raw, &JsValue::from_str("publicKey"))
        .ok()
        .filter(|v| v.is_object())
        .unwrap_or_else(|| raw.clone())
}

fn prepare_create_public_key(raw: &JsValue) -> JsValue {
    let pk = unwrap_public_key_options(raw);

    if let Some(ch) = Reflect::get(&pk, &JsValue::from_str("challenge"))
        .ok()
        .and_then(|v| v.as_string())
    {
        let _ = Reflect::set(
            &pk,
            &JsValue::from_str("challenge"),
            &b64url_to_uint8_array(&ch),
        );
    }
    if let Ok(user) = Reflect::get(&pk, &JsValue::from_str("user")) {
        if let Some(uid) = Reflect::get(&user, &JsValue::from_str("id"))
            .ok()
            .and_then(|v| v.as_string())
        {
            let _ = Reflect::set(
                &user,
                &JsValue::from_str("id"),
                &b64url_to_uint8_array(&uid),
            );
        }
    }
    decode_credential_descriptor_ids(&pk, "excludeCredentials");

    // #51 invariants: omit authenticatorAttachment, attestation = "none", userVerification = "preferred".
    let _ = Reflect::set(
        &pk,
        &JsValue::from_str("attestation"),
        &JsValue::from_str("none"),
    );
    let auth_sel = Reflect::get(&pk, &JsValue::from_str("authenticatorSelection"))
        .ok()
        .filter(|v| v.is_object())
        .unwrap_or_else(|| Object::new().into());
    if let Ok(auth_obj) = auth_sel.clone().dyn_into::<Object>() {
        let _ = Reflect::delete_property(&auth_obj, &JsValue::from_str("authenticatorAttachment"));
    }
    let _ = Reflect::set(
        &auth_sel,
        &JsValue::from_str("userVerification"),
        &JsValue::from_str("preferred"),
    );
    let _ = Reflect::set(&pk, &JsValue::from_str("authenticatorSelection"), &auth_sel);

    let wrapper = Object::new();
    let _ = Reflect::set(&wrapper, &JsValue::from_str("publicKey"), &pk);
    wrapper.into()
}

fn prepare_get_public_key(raw: &JsValue) -> JsValue {
    let pk = unwrap_public_key_options(raw);
    if let Some(ch) = Reflect::get(&pk, &JsValue::from_str("challenge"))
        .ok()
        .and_then(|v| v.as_string())
    {
        let _ = Reflect::set(
            &pk,
            &JsValue::from_str("challenge"),
            &b64url_to_uint8_array(&ch),
        );
    }
    decode_credential_descriptor_ids(&pk, "allowCredentials");
    let _ = Reflect::set(
        &pk,
        &JsValue::from_str("userVerification"),
        &JsValue::from_str("preferred"),
    );

    let wrapper = Object::new();
    let _ = Reflect::set(&wrapper, &JsValue::from_str("publicKey"), &pk);
    wrapper.into()
}

fn client_extension_results_obj(cred: &JsValue) -> JsValue {
    if let Ok(get_ext) = Reflect::get(cred, &JsValue::from_str("getClientExtensionResults")) {
        if let Ok(fn_ref) = get_ext.dyn_into::<Function>() {
            if let Ok(res) = fn_ref.call0(cred) {
                if res.is_object() {
                    return res;
                }
            }
        }
    }
    Object::new().into()
}

fn serialize_attestation_json(cred: &JsValue) -> Option<String> {
    let out = Object::new();
    let id = Reflect::get(cred, &JsValue::from_str("id")).ok()?;
    let typ = Reflect::get(cred, &JsValue::from_str("type"))
        .ok()
        .filter(|v| v.is_string())
        .unwrap_or_else(|| JsValue::from_str("public-key"));
    let raw_id = js_buffer_prop_to_b64url(cred, "rawId").unwrap_or_default();
    let resp = Reflect::get(cred, &JsValue::from_str("response")).ok()?;
    let client_data = js_buffer_prop_to_b64url(&resp, "clientDataJSON").unwrap_or_default();
    let att_obj = js_buffer_prop_to_b64url(&resp, "attestationObject").unwrap_or_default();

    let resp_out = Object::new();
    let _ = Reflect::set(
        &resp_out,
        &JsValue::from_str("clientDataJSON"),
        &JsValue::from_str(&client_data),
    );
    let _ = Reflect::set(
        &resp_out,
        &JsValue::from_str("attestationObject"),
        &JsValue::from_str(&att_obj),
    );
    if let Ok(get_transports) = Reflect::get(&resp, &JsValue::from_str("getTransports")) {
        if let Ok(fn_ref) = get_transports.dyn_into::<Function>() {
            if let Ok(transports) = fn_ref.call0(&resp) {
                let _ = Reflect::set(&resp_out, &JsValue::from_str("transports"), &transports);
            }
        }
    }

    let _ = Reflect::set(&out, &JsValue::from_str("id"), &id);
    let _ = Reflect::set(
        &out,
        &JsValue::from_str("rawId"),
        &JsValue::from_str(&raw_id),
    );
    let _ = Reflect::set(&out, &JsValue::from_str("type"), &typ);
    let _ = Reflect::set(&out, &JsValue::from_str("response"), &resp_out);
    let _ = Reflect::set(
        &out,
        &JsValue::from_str("extensions"),
        &client_extension_results_obj(cred),
    );
    JSON::stringify(&out).ok().and_then(|s| s.as_string())
}

fn serialize_assertion_json(cred: &JsValue) -> Option<String> {
    let out = Object::new();
    let id = Reflect::get(cred, &JsValue::from_str("id")).ok()?;
    let typ = Reflect::get(cred, &JsValue::from_str("type"))
        .ok()
        .filter(|v| v.is_string())
        .unwrap_or_else(|| JsValue::from_str("public-key"));
    let raw_id = js_buffer_prop_to_b64url(cred, "rawId").unwrap_or_default();
    let resp = Reflect::get(cred, &JsValue::from_str("response")).ok()?;

    let resp_out = Object::new();
    if let Some(cd) = js_buffer_prop_to_b64url(&resp, "clientDataJSON") {
        let _ = Reflect::set(
            &resp_out,
            &JsValue::from_str("clientDataJSON"),
            &JsValue::from_str(&cd),
        );
    }
    if let Some(ad) = js_buffer_prop_to_b64url(&resp, "authenticatorData") {
        let _ = Reflect::set(
            &resp_out,
            &JsValue::from_str("authenticatorData"),
            &JsValue::from_str(&ad),
        );
    }
    if let Some(sig) = js_buffer_prop_to_b64url(&resp, "signature") {
        let _ = Reflect::set(
            &resp_out,
            &JsValue::from_str("signature"),
            &JsValue::from_str(&sig),
        );
    }
    if let Some(uh) = js_buffer_prop_to_b64url(&resp, "userHandle") {
        let _ = Reflect::set(
            &resp_out,
            &JsValue::from_str("userHandle"),
            &JsValue::from_str(&uh),
        );
    }

    let _ = Reflect::set(&out, &JsValue::from_str("id"), &id);
    let _ = Reflect::set(
        &out,
        &JsValue::from_str("rawId"),
        &JsValue::from_str(&raw_id),
    );
    let _ = Reflect::set(&out, &JsValue::from_str("type"), &typ);
    let _ = Reflect::set(&out, &JsValue::from_str("response"), &resp_out);
    let _ = Reflect::set(
        &out,
        &JsValue::from_str("extensions"),
        &client_extension_results_obj(cred),
    );
    JSON::stringify(&out).ok().and_then(|s| s.as_string())
}

fn promise_then(
    p: &Promise,
    on_ok: Closure<dyn FnMut(JsValue) -> JsValue>,
) -> Result<Promise, String> {
    let then_fn = Reflect::get(p.as_ref(), &JsValue::from_str("then"))
        .ok()
        .and_then(|v| v.dyn_into::<Function>().ok())
        .ok_or_else(|| "Promise missing .then".to_owned())?;
    let next = then_fn
        .call1(p.as_ref(), on_ok.as_ref())
        .ok()
        .and_then(|v| v.dyn_into::<Promise>().ok())
        .ok_or_else(|| "Promise .then invocation failed".to_owned())?;
    on_ok.forget();
    Ok(next)
}

fn fetch_json_post(url: &str, body_json: &str) -> Result<Promise, String> {
    let win = web_sys::window().ok_or_else(|| "Window unavailable".to_owned())?;
    let fetch_fn = Reflect::get(&win, &JsValue::from_str("fetch"))
        .ok()
        .and_then(|v| v.dyn_into::<Function>().ok())
        .ok_or_else(|| "window.fetch unavailable".to_owned())?;

    let headers = Object::new();
    let _ = Reflect::set(
        &headers,
        &JsValue::from_str("Content-Type"),
        &JsValue::from_str("application/json"),
    );

    let init = Object::new();
    let _ = Reflect::set(
        &init,
        &JsValue::from_str("method"),
        &JsValue::from_str("POST"),
    );
    let _ = Reflect::set(
        &init,
        &JsValue::from_str("credentials"),
        &JsValue::from_str("same-origin"),
    );
    let _ = Reflect::set(&init, &JsValue::from_str("headers"), &headers);
    let _ = Reflect::set(
        &init,
        &JsValue::from_str("body"),
        &JsValue::from_str(body_json),
    );

    let resp_promise = fetch_fn
        .call2(&win, &JsValue::from_str(url), &init)
        .ok()
        .and_then(|v| v.dyn_into::<Promise>().ok())
        .ok_or_else(|| format!("POST {url} failed to start"))?;

    let url_owned = url.to_owned();
    let on_resp: Closure<dyn FnMut(JsValue) -> JsValue> =
        Closure::once(move |resp: JsValue| -> JsValue {
            let ok = Reflect::get(&resp, &JsValue::from_str("ok"))
                .ok()
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if !ok {
                let status = Reflect::get(&resp, &JsValue::from_str("status"))
                    .ok()
                    .and_then(|v| v.as_f64())
                    .map_or(0_u16, |n| n as u16);
                return Promise::reject(&JsValue::from_str(&format!(
                    "HTTP {status} from {url_owned}"
                )))
                .into();
            }
            if let Ok(json_fn) = Reflect::get(&resp, &JsValue::from_str("json"))
                .and_then(|v| v.dyn_into::<Function>())
            {
                if let Ok(p) = json_fn.call0(&resp) {
                    return p;
                }
            }
            Promise::reject(&JsValue::from_str("Invalid JSON response")).into()
        });

    promise_then(&resp_promise, on_resp)
}

fn start_webauthn_ceremony(
    creds: &JsValue,
    is_create: bool,
    start_url: &str,
    start_body: &str,
) -> Result<Promise, String> {
    let options_promise = fetch_json_post(start_url, start_body)?;
    let method_name = if is_create { "create" } else { "get" };
    let cred_fn = Reflect::get(creds, &JsValue::from_str(method_name))
        .and_then(|v| v.dyn_into::<Function>())
        .map_err(|_| format!("navigator.credentials.{method_name} unavailable"))?;

    let creds_for_then = creds.clone();
    let on_options: Closure<dyn FnMut(JsValue) -> JsValue> =
        Closure::once(move |opts: JsValue| -> JsValue {
            let challenge_id = extract_challenge_id(&opts);
            let prepared = if is_create {
                prepare_create_public_key(&opts)
            } else {
                prepare_get_public_key(&opts)
            };
            let cred_promise = match cred_fn.call1(&creds_for_then, &prepared) {
                Ok(v) => match v.dyn_into::<Promise>() {
                    Ok(p) => p,
                    Err(_) => {
                        return Promise::reject(&JsValue::from_str(
                            "WebAuthn API did not return a Promise",
                        ))
                        .into();
                    }
                },
                Err(e) => return Promise::reject(&e).into(),
            };
            let on_cred: Closure<dyn FnMut(JsValue) -> JsValue> =
                Closure::once(move |cred: JsValue| -> JsValue {
                    let serialized = if is_create {
                        serialize_attestation_json(&cred)
                    } else {
                        serialize_assertion_json(&cred)
                    };
                    let Some(cred_json) = serialized else {
                        return Promise::reject(&JsValue::from_str(
                            "Failed to serialize WebAuthn credential",
                        ))
                        .into();
                    };
                    let pair = Array::new();
                    pair.push(&challenge_id.map_or(JsValue::NULL, |c| JsValue::from_str(&c)));
                    pair.push(&JsValue::from_str(&cred_json));
                    pair.into()
                });
            match promise_then(&cred_promise, on_cred) {
                Ok(next) => next.into(),
                Err(msg) => Promise::reject(&JsValue::from_str(&msg)).into(),
            }
        });

    promise_then(&options_promise, on_options)
}

fn unpack_ceremony_pair(pair: &JsValue) -> Option<(Option<String>, String)> {
    let arr = pair.dyn_ref::<Array>()?;
    let challenge_id = arr.get(0).as_string();
    let cred_json = arr.get(1).as_string()?;
    Some((challenge_id, cred_json))
}

fn js_error_message(err: &JsValue) -> String {
    if let Some(s) = err.as_string() {
        return s;
    }
    let name = Reflect::get(err, &JsValue::from_str("name"))
        .ok()
        .and_then(|v| v.as_string());
    let msg = Reflect::get(err, &JsValue::from_str("message"))
        .ok()
        .and_then(|v| v.as_string());
    match (name, msg) {
        (Some(n), Some(m)) if !m.is_empty() => format!("{n}: {m}"),
        (_, Some(m)) if !m.is_empty() => m,
        (Some(n), _) if !n.is_empty() => n,
        _ => JSON::stringify(err)
            .ok()
            .and_then(|s| s.as_string())
            .unwrap_or_else(|| "WebAuthn ceremony rejected".to_owned()),
    }
}

fn report_webauthn_error(state: &Rc<RefCell<ConsoleState>>, msg: &str) {
    console::warn_2(
        &JsValue::from_str("muconsole: WebAuthn ceremony failed"),
        &JsValue::from_str(msg),
    );
    if let Ok(mut st) = state.try_borrow_mut() {
        st.on_webauthn_error(msg);
        repaint(&st);
    }
}

fn spawn_webauthn(state: Rc<RefCell<ConsoleState>>, worker: Worker, req: WebAuthnRequest) {
    let Some(win) = web_sys::window() else {
        return;
    };
    let origin = win.location().origin().unwrap_or_default();
    let Ok(nav) = Reflect::get(&win, &JsValue::from_str("navigator")) else {
        report_webauthn_error(&state, "Browser navigator unavailable");
        return;
    };
    let Ok(creds) = Reflect::get(&nav, &JsValue::from_str("credentials")) else {
        report_webauthn_error(&state, "WebAuthn unavailable in this browser context");
        return;
    };
    if creds.is_undefined() || creds.is_null() {
        report_webauthn_error(&state, "WebAuthn unavailable in this browser context");
        return;
    }

    let reg_url = format!("{origin}{}", paths::AUTH_PASSKEYS_REGISTER_START);
    let elevate_url = format!("{origin}{}", paths::AUTH_ELEVATE_START);

    let ceremony_promise = match &req {
        WebAuthnRequest::RegisterPasskey {
            label,
            requires_step_up: false,
        } => {
            let body = register_start_body(label.as_deref(), None, None);
            start_webauthn_ceremony(&creds, true, &reg_url, &body)
        }
        WebAuthnRequest::RegisterPasskey {
            label,
            requires_step_up: true,
        } => {
            let label_for_step = label.clone();
            let creds_for_step = creds.clone();
            let reg_url_for_step = reg_url;
            start_webauthn_ceremony(&creds, false, &elevate_url, passkey_manage_start_body())
                .and_then(|step_promise| {
                    let on_step_up: Closure<dyn FnMut(JsValue) -> JsValue> =
                        Closure::once(move |pair: JsValue| -> JsValue {
                            let Some((step_cid, step_cred_json)) = unpack_ceremony_pair(&pair)
                            else {
                                return Promise::reject(&JsValue::from_str(
                                    "Missing step-up assertion payload",
                                ))
                                .into();
                            };
                            let body = register_start_body(
                                label_for_step.as_deref(),
                                step_cid.as_deref(),
                                Some(&step_cred_json),
                            );
                            match start_webauthn_ceremony(
                                &creds_for_step,
                                true,
                                &reg_url_for_step,
                                &body,
                            ) {
                                Ok(next) => next.into(),
                                Err(msg) => Promise::reject(&JsValue::from_str(&msg)).into(),
                            }
                        });
                    promise_then(&step_promise, on_step_up)
                })
        }
        WebAuthnRequest::ElevateTimedRole { role_id } => {
            start_webauthn_ceremony(&creds, false, &elevate_url, &elevate_start_body(role_id))
        }
        WebAuthnRequest::RemovePasskey { .. } => {
            start_webauthn_ceremony(&creds, false, &elevate_url, passkey_manage_start_body())
        }
    };

    let ceremony_promise = match ceremony_promise {
        Ok(p) => p,
        Err(msg) => {
            report_webauthn_error(&state, &msg);
            return;
        }
    };

    let worker_for_done = worker;
    let state_for_done = Rc::clone(&state);
    let on_done: Closure<dyn FnMut(JsValue)> = Closure::once(move |pair: JsValue| {
        let Some((challenge_id, cred_json)) = unpack_ceremony_pair(&pair) else {
            report_webauthn_error(&state_for_done, "Missing WebAuthn credential payload");
            return;
        };
        let envelope = wrap_credential_envelope_json(challenge_id.as_deref(), &cred_json);
        match req {
            WebAuthnRequest::RegisterPasskey { label, .. } => send(
                &worker_for_done,
                &UiCommand::EnrollPasskey {
                    label,
                    attestation_json: envelope,
                },
            ),
            WebAuthnRequest::ElevateTimedRole { role_id } => send(
                &worker_for_done,
                &UiCommand::ElevateWithPasskey {
                    role_id,
                    assertion_json: envelope,
                },
            ),
            WebAuthnRequest::RemovePasskey { credential_id } => send(
                &worker_for_done,
                &UiCommand::RemovePasskey {
                    credential_id,
                    step_up_assertion_json: envelope,
                },
            ),
        }
    });
    let state_for_err = state;
    let on_err: Closure<dyn FnMut(JsValue)> = Closure::once(move |err: JsValue| {
        let msg = js_error_message(&err);
        report_webauthn_error(&state_for_err, &msg);
    });

    let _ = ceremony_promise.then2(&on_done, &on_err);
    on_done.forget();
    on_err.forget();
}

fn trap_dialog_tab(doc: &web_sys::Document, ev: &KeyboardEvent) -> bool {
    let Ok(qsa_fn) = Reflect::get(doc.as_ref(), &JsValue::from_str("querySelectorAll"))
        .and_then(|v| v.dyn_into::<Function>())
    else {
        return false;
    };
    let selector = JsValue::from_str(
        "#muconsole-dashboard dialog[open] button:not([disabled]), \
         #muconsole-dashboard dialog[open] input:not([disabled]), \
         #muconsole-dashboard dialog[open] a[href], \
         #muconsole-dashboard dialog[open] select:not([disabled]), \
         #muconsole-dashboard dialog[open] textarea:not([disabled]), \
         #muconsole-dashboard dialog[open] [tabindex]:not([tabindex='-1']), \
         #muconsole-dashboard [role='dialog'] button:not([disabled]), \
         #muconsole-dashboard [role='dialog'] input:not([disabled]), \
         #muconsole-dashboard [role='dialog'] a[href], \
         #muconsole-dashboard [role='dialog'] select:not([disabled]), \
         #muconsole-dashboard [role='dialog'] textarea:not([disabled]), \
         #muconsole-dashboard [role='dialog'] [tabindex]:not([tabindex='-1'])",
    );
    let Ok(node_list) = qsa_fn.call1(doc.as_ref(), &selector) else {
        return false;
    };
    let list = Array::from(&node_list);
    let len = list.length();
    if len == 0 {
        return false;
    }
    let first = list.get(0).dyn_into::<HtmlElement>().ok();
    let last = list.get(len - 1).dyn_into::<HtmlElement>().ok();
    let active = doc
        .active_element()
        .and_then(|e| e.dyn_into::<HtmlElement>().ok());

    ev.prevent_default();
    if ev.shift_key() {
        let on_first = active
            .as_ref()
            .zip(first.as_ref())
            .is_some_and(|(a, f)| Object::is(a.as_ref(), f.as_ref()));
        if on_first || active.is_none() {
            if let Some(el) = last {
                let _ = el.focus();
            }
        } else {
            for idx in (1..len).rev() {
                if active
                    .as_ref()
                    .is_some_and(|a| Object::is(&list.get(idx), a.as_ref()))
                {
                    if let Ok(prev) = list.get(idx - 1).dyn_into::<HtmlElement>() {
                        let _ = prev.focus();
                    }
                    break;
                }
            }
        }
    } else {
        let on_last = active
            .as_ref()
            .zip(last.as_ref())
            .is_some_and(|(a, l)| Object::is(a.as_ref(), l.as_ref()));
        if on_last || active.is_none() {
            if let Some(el) = first {
                let _ = el.focus();
            }
        } else {
            for idx in 0..(len - 1) {
                if active
                    .as_ref()
                    .is_some_and(|a| Object::is(&list.get(idx), a.as_ref()))
                {
                    if let Ok(next) = list.get(idx + 1).dyn_into::<HtmlElement>() {
                        let _ = next.focus();
                    }
                    break;
                }
            }
        }
    }
    true
}

/// Spawn the worker and wire the message and DOM interaction bridges.
pub fn start() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let location = window.location();
    let raw_search = location.search().unwrap_or_default();
    let mut hash = location.hash().unwrap_or_default();

    let (invite_from_url, clean_search) = extract_invite_token(&raw_search);
    if let Some(ref token) = invite_from_url {
        if let Ok(Some(storage)) = window.session_storage() {
            let _ = storage.set_item(PENDING_INVITE_STORAGE_KEY, token);
        }
        if let Ok(history) = window.history() {
            let pathname = location.pathname().unwrap_or_else(|_| "/".into());
            let clean_url = format!("{pathname}{clean_search}{hash}");
            let _ = history.replace_state_with_url(&JsValue::NULL, "", Some(&clean_url));
        }
    }

    let stored_invite = invite_from_url.or_else(|| {
        window
            .session_storage()
            .ok()
            .flatten()
            .and_then(|s| s.get_item(PENDING_INVITE_STORAGE_KEY).ok().flatten())
            .filter(|t| !t.is_empty())
    });

    let stored_pending_hash = window
        .session_storage()
        .ok()
        .flatten()
        .and_then(|s| s.get_item(PENDING_HASH_STORAGE_KEY).ok().flatten())
        .filter(|h| has_explicit_route(h));
    if !has_explicit_route(&hash) {
        if let Some(ref saved) = stored_pending_hash {
            hash.clone_from(saved);
        }
    }

    let cfg = WorkerConfig::from_query(&clean_search, &location.origin().unwrap_or_default());
    let state = Rc::new(RefCell::new(ConsoleState::new()));
    {
        let stored_font = window
            .local_storage()
            .ok()
            .flatten()
            .and_then(|s| s.get_item(FONT_STORAGE_KEY).ok().flatten());
        let stored_skin = window
            .local_storage()
            .ok()
            .flatten()
            .and_then(|s| s.get_item(SKIN_STORAGE_KEY).ok().flatten());
        let stored_repo = window
            .local_storage()
            .ok()
            .flatten()
            .and_then(|s| s.get_item(REPO_STORAGE_KEY).ok().flatten());

        let mut st = state.borrow_mut();
        st.set_viewport(window_viewport(&window), SafeArea::ZERO);
        st.init_from_env(&hash, &raw_search, stored_font.as_deref());
        st.restore_stored_skin(&raw_search, stored_skin.as_deref());
        if st.pending_invite.is_none() {
            st.pending_invite.clone_from(&stored_invite);
        }
        if st.stashed_deep_link.is_none() {
            st.stashed_deep_link = stored_pending_hash;
        }
        st.restore_stored_repo(stored_repo.as_deref());
        sync_browser_state(&mut st);
        repaint(&st);
    }

    let worker = match Worker::new(WORKER_URL) {
        Ok(w) => w,
        Err(e) => {
            console::error_2(&JsValue::from_str("muconsole: worker spawn failed"), &e);
            return;
        }
    };

    {
        let st = state.borrow();
        send(&worker, &UiCommand::Configure(cfg));
        if let Some(token) = stored_invite {
            send(&worker, &UiCommand::RedeemInvite { token });
        }
        send(&worker, &UiCommand::Subscribe(st.route.clone()));
        send(&worker, &UiCommand::SelectSubView(st.route.subview));
    }

    let state_for_msg = Rc::clone(&state);
    let worker_for_msg = worker.clone();
    let onmessage = Closure::<dyn FnMut(MessageEvent)>::new(move |ev: MessageEvent| {
        let data = ev.data();
        let bytes = data
            .dyn_ref::<ArrayBuffer>()
            .map(|b| Uint8Array::new(b).to_vec())
            .unwrap_or_default();
        let mut st = state_for_msg.borrow_mut();
        match st.on_worker_frame(&bytes) {
            ClientUpdate::ReloadRequired { expected, found } => {
                show_banner(&reload_banner_text(expected, found));
            }
            ClientUpdate::View => {
                if st.is_operator() && st.pending_invite.take().is_some() {
                    if let Some(Ok(Some(storage))) = web_sys::window().map(|w| w.session_storage())
                    {
                        let _ = storage.remove_item(PENDING_INVITE_STORAGE_KEY);
                    }
                }
                if st.route_sync_dirty {
                    send(&worker_for_msg, &UiCommand::Subscribe(st.route.clone()));
                    send(&worker_for_msg, &UiCommand::SelectSubView(st.route.subview));
                }
                sync_browser_state(&mut st);
                repaint(&st);
            }
            ClientUpdate::Ready(_)
            | ClientUpdate::Connection(_)
            | ClientUpdate::Badges(_)
            | ClientUpdate::Error(_) => {
                repaint(&st);
            }
            ClientUpdate::Heartbeat => {}
        }
    });
    worker.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
    onmessage.forget();

    // Window resize listener.
    {
        let state_for_resize = Rc::clone(&state);
        let on_resize = Closure::<dyn FnMut()>::new(move || {
            let mut st = state_for_resize.borrow_mut();
            if let Some(win) = web_sys::window() {
                st.set_viewport(window_viewport(&win), SafeArea::ZERO);
            }
            repaint(&st);
        });
        let _ =
            window.add_event_listener_with_callback("resize", on_resize.as_ref().unchecked_ref());
        on_resize.forget();
    }

    // Window hashchange listener.
    {
        let state_for_hash = Rc::clone(&state);
        let worker_for_hash = worker.clone();
        let on_hashchange = Closure::<dyn FnMut()>::new(move || {
            let Some(win) = web_sys::window() else {
                return;
            };
            let hash = win.location().hash().unwrap_or_default();
            let mut st = state_for_hash.borrow_mut();
            if format_hash(&st.route) != hash {
                st.apply_hash(&hash);
                let cmds = vec![
                    UiCommand::Subscribe(st.route.clone()),
                    UiCommand::SelectSubView(st.route.subview),
                ];
                send_all(&worker_for_hash, &cmds);
                sync_browser_state(&mut st);
                repaint(&st);
            }
        });
        let _ = window
            .add_event_listener_with_callback("hashchange", on_hashchange.as_ref().unchecked_ref());
        on_hashchange.forget();
    }

    // Document DOM interactions.
    if let Some(doc) = window.document() {
        // Explicit user pointer activity resets Timed idle_timer (passive hover does not).
        {
            let state_for_ptr = Rc::clone(&state);
            let worker_for_ptr = worker.clone();
            let on_pointer = Closure::<dyn FnMut(Event)>::new(move |_ev: Event| {
                let mut st = state_for_ptr.borrow_mut();
                let cmds = st.on_explicit_user_activity(now_ms());
                if !cmds.is_empty() {
                    send_all(&worker_for_ptr, &cmds);
                    repaint(&st);
                }
            });
            let _ = doc.add_event_listener_with_callback(
                "pointerdown",
                on_pointer.as_ref().unchecked_ref(),
            );
            on_pointer.forget();
        }

        // Click delegation.
        {
            let state_for_click = Rc::clone(&state);
            let worker_for_click = worker.clone();
            let on_click = Closure::<dyn FnMut(MouseEvent)>::new(move |ev: MouseEvent| {
                let Some(target) = ev.target().and_then(|t| t.dyn_into::<Element>().ok()) else {
                    return;
                };
                if let Some((attr, val)) = find_action_attribute(&target) {
                    let maybe_webauthn = {
                        let mut st = state_for_click.borrow_mut();
                        let now = now_ms();
                        let mut cmds = st.on_explicit_user_activity(now);
                        let action_cmds = st.on_dom_action(attr, &val, ev.shift_key(), now);
                        cmds.extend(action_cmds);
                        let req = handle_post_action_effects(&mut st, &cmds);
                        send_all(&worker_for_click, &cmds);
                        sync_browser_state(&mut st);
                        repaint(&st);
                        req
                    };
                    if let Some(req) = maybe_webauthn {
                        spawn_webauthn(Rc::clone(&state_for_click), worker_for_click.clone(), req);
                    }
                }
            });
            let _ =
                doc.add_event_listener_with_callback("click", on_click.as_ref().unchecked_ref());
            on_click.forget();
        }

        // Keyboard + modal focus trap + Escape gate + explicit user activity.
        {
            let state_for_key = Rc::clone(&state);
            let worker_for_key = worker.clone();
            let on_keydown = Closure::<dyn FnMut(KeyboardEvent)>::new(move |ev: KeyboardEvent| {
                let key = ev.key();
                let target_id = ev
                    .target()
                    .and_then(|t| t.dyn_into::<Element>().ok())
                    .map(|el| el.id())
                    .unwrap_or_default();
                let target_is_input = matches!(
                    target_id.as_str(),
                    "muconsole-filter"
                        | "muconsole-profile-display-name"
                        | "muconsole-ab-rollback-note"
                        | "muconsole-risk-phrase"
                );

                {
                    let mut st = state_for_key.borrow_mut();
                    let now = now_ms();
                    let activity_cmds = st.on_explicit_user_activity(now);
                    if !activity_cmds.is_empty() {
                        send_all(&worker_for_key, &activity_cmds);
                    }

                    if st.active_dialog().is_some() {
                        if key == "Tab" {
                            if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                                let _ = trap_dialog_tab(&doc, &ev);
                            }
                            return;
                        }
                        if key == "Escape" {
                            ev.prevent_default();
                            if let Some(cmd) = st.on_key("Escape", ev.shift_key(), now) {
                                send(&worker_for_key, &cmd);
                            }
                            sync_browser_state(&mut st);
                            repaint(&st);
                            return;
                        }
                    }
                }

                if target_is_input && key != "Escape" {
                    if key == "Enter" && target_id == "muconsole-filter" {
                        let st = state_for_key.borrow();
                        send(&worker_for_key, &UiCommand::Subscribe(st.route.clone()));
                    }
                    return;
                }

                if key == "/" && !target_is_input {
                    ev.prevent_default();
                    if let Some(input) = web_sys::window()
                        .and_then(|w| w.document())
                        .and_then(|d| d.get_element_by_id("muconsole-filter"))
                        .and_then(|e| e.dyn_into::<HtmlInputElement>().ok())
                    {
                        let _ = input.focus();
                    }
                }

                let mut st = state_for_key.borrow_mut();
                if let Some(cmd) = st.on_key(&key, ev.shift_key(), now_ms()) {
                    send(&worker_for_key, &cmd);
                }
                if key == "Escape" && target_is_input {
                    if let Some(input) = web_sys::window()
                        .and_then(|w| w.document())
                        .and_then(|d| d.get_element_by_id(&target_id))
                        .and_then(|e| e.dyn_into::<HtmlInputElement>().ok())
                    {
                        let _ = input.blur();
                    }
                }
                sync_browser_state(&mut st);
                repaint(&st);
            });
            let _ = doc
                .add_event_listener_with_callback("keydown", on_keydown.as_ref().unchecked_ref());
            on_keydown.forget();
        }

        // Input bindings: filter box + modal inputs (`display_name`, A|B rollback note, risk confirm).
        {
            let state_for_input = Rc::clone(&state);
            let on_input = Closure::<dyn FnMut(Event)>::new(move |ev: Event| {
                let Some(input) = ev
                    .target()
                    .and_then(|t| t.dyn_into::<HtmlInputElement>().ok())
                else {
                    return;
                };
                let id = input.id();
                let val = input.value();
                let mut st = state_for_input.borrow_mut();
                match id.as_str() {
                    "muconsole-filter" => {
                        st.set_filter_query(&val, now_ms());
                        sync_browser_state(&mut st);
                        repaint(&st);
                    }
                    "muconsole-profile-display-name" => {
                        st.profile_display_name_draft = val;
                    }
                    "muconsole-ab-rollback-note" => {
                        st.platform_form.ab_rollback_note = val;
                    }
                    "muconsole-risk-phrase" => {
                        st.set_risk_confirm_input(&val);
                        repaint(&st);
                    }
                    _ => {}
                }
            });
            let _ =
                doc.add_event_listener_with_callback("input", on_input.as_ref().unchecked_ref());
            on_input.forget();
        }

        // Wheel pagination on selector bars and data screens.
        {
            let state_for_wheel = Rc::clone(&state);
            let on_wheel = Closure::<dyn FnMut(WheelEvent)>::new(move |ev: WheelEvent| {
                let Some(target) = ev.target().and_then(|t| t.dyn_into::<Element>().ok()) else {
                    return;
                };
                let in_top_bar = target
                    .closest("[data-control-id='c2-top-bar'], [data-repo-step]")
                    .ok()
                    .flatten()
                    .is_some();
                let in_screen = target
                    .closest("[data-cols][data-rows]")
                    .ok()
                    .flatten()
                    .is_some();
                if !in_top_bar && !in_screen {
                    return;
                }
                ev.prevent_default();
                let delta = if ev.delta_y() > 0.0 { 1 } else { -1 };
                let mut st = state_for_wheel.borrow_mut();
                let now = now_ms();
                let _ = st.on_explicit_user_activity(now);
                if in_top_bar {
                    if ev.shift_key() {
                        st.rotate_inner(delta, now);
                    } else {
                        st.rotate_outer(delta, now);
                    }
                } else if target
                    .closest("[data-control-id='ci-job-detail-screen']")
                    .ok()
                    .flatten()
                    .is_some()
                {
                    let _ = st.activate_target(if delta < 0 {
                        "ci-log-page-prev"
                    } else {
                        "ci-log-page-next"
                    });
                } else {
                    st.step_active_page(delta);
                }
                sync_browser_state(&mut st);
                repaint(&st);
            });
            let _ =
                doc.add_event_listener_with_callback("wheel", on_wheel.as_ref().unchecked_ref());
            on_wheel.forget();
        }

        // Tab visibility throttling.
        {
            let w = worker.clone();
            let on_visibility = Closure::<dyn FnMut()>::new(move || {
                let hidden = web_sys::window()
                    .and_then(|w| w.document())
                    .is_some_and(|d| d.hidden());
                send(&w, &UiCommand::SetVisibility { hidden });
            });
            let _ = doc.add_event_listener_with_callback(
                "visibilitychange",
                on_visibility.as_ref().unchecked_ref(),
            );
            on_visibility.forget();
        }
    }

    // Periodic 50ms rotary debounce settle timer.
    {
        let state_for_rotary = Rc::clone(&state);
        let worker_for_rotary = worker.clone();
        let on_rotary_tick = Closure::<dyn FnMut()>::new(move || {
            let mut st = state_for_rotary.borrow_mut();
            if st.pending_select_at_ms.is_some() {
                let cmds = st.tick(now_ms());
                if !cmds.is_empty() {
                    send_all(&worker_for_rotary, &cmds);
                    sync_browser_state(&mut st);
                    repaint(&st);
                }
            }
        });
        let _ = window.set_interval_with_callback_and_timeout_and_arguments_0(
            on_rotary_tick.as_ref().unchecked_ref(),
            50,
        );
        on_rotary_tick.forget();
    }

    // 1-second local countdown timer for `+role[seconds_remaining][idle_timer]` and 00:00 expiry.
    {
        let state_for_lease = Rc::clone(&state);
        let worker_for_lease = worker.clone();
        let on_lease_tick = Closure::<dyn FnMut()>::new(move || {
            let mut st = state_for_lease.borrow_mut();
            let has_leases = !st.auth().leases.is_empty();
            let cmds = st.tick(now_ms());
            if !cmds.is_empty() || has_leases {
                if !cmds.is_empty() {
                    send_all(&worker_for_lease, &cmds);
                }
                if st.route_sync_dirty {
                    send(&worker_for_lease, &UiCommand::Subscribe(st.route.clone()));
                    send(
                        &worker_for_lease,
                        &UiCommand::SelectSubView(st.route.subview),
                    );
                }
                sync_browser_state(&mut st);
                repaint(&st);
            }
        });
        let _ = window.set_interval_with_callback_and_timeout_and_arguments_0(
            on_lease_tick.as_ref().unchecked_ref(),
            1_000,
        );
        on_lease_tick.forget();
    }

    WORKER.with(|slot| *slot.borrow_mut() = Some(worker));
}
