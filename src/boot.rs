// SPDX-License-Identifier: AGPL-3.0-only
//! Main-thread bootstrap (wasm32 only): spawn the data worker, forward
//! configuration and visibility, bind DOM click/key/input/hash/wheel events
//! across all 5 Areas and 17 Sub-Views, and show the "reload required"
//! banner on a protocol mismatch.

use std::cell::RefCell;
use std::rc::Rc;

use js_sys::{Array, ArrayBuffer, Date, Uint8Array};
use mu_console_layout::{SafeArea, Viewport};
use mu_console_protocol::{AuthState, ConsoleSkin, UiCommand, WorkerConfig};
use mu_console_theme::skin::Skin;
use mu_console_theme::wireframe::WireframeSkin;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{
    console, Element, Event, HtmlInputElement, KeyboardEvent, MessageEvent, MouseEvent, WheelEvent,
    Worker,
};

use crate::client::{reload_banner_text, ClientUpdate, DataClient, WORKER_URL};
use crate::components::{filter_placeholder, render_console, render_html_shell};
use crate::router::{area_slug, format_hash};
use crate::state::{
    extract_invite_token, font_slug, is_action_root_boundary, is_passive_container_control_id,
    skin_slug, ConsoleState, ACTION_ATTRS, FONT_STORAGE_KEY, PENDING_INVITE_STORAGE_KEY,
    REPO_STORAGE_KEY, SKIN_STORAGE_KEY,
};

thread_local! {
    static WORKER: RefCell<Option<Worker>> = const { RefCell::new(None) };
    static LAST_SVG: RefCell<String> = const { RefCell::new(String::new()) };
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

fn sync_browser_state(st: &ConsoleState) {
    let Some(win) = web_sys::window() else {
        return;
    };
    let desired_hash = format_hash(&st.route);
    let current_hash = win.location().hash().unwrap_or_default();
    if current_hash != desired_hash {
        let _ = win.location().set_hash(&desired_hash);
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

fn repaint(st: &ConsoleState) {
    let Some(win) = web_sys::window() else {
        return;
    };
    let Some(doc) = win.document() else {
        return;
    };
    let vp = window_viewport(&win);
    let skin: &dyn Skin = &WireframeSkin;
    let svg = render_console(skin, st, vp, SafeArea::ZERO);

    if let Some(dash) = doc.get_element_by_id("muconsole-dashboard") {
        let svg_changed = LAST_SVG.with(|last| {
            let mut prev = last.borrow_mut();
            if *prev == svg {
                false
            } else {
                prev.clone_from(&svg);
                true
            }
        });
        if svg_changed {
            dash.set_inner_html(&svg);
        }
        if let Some(shell) = doc.get_element_by_id("muconsole-shell") {
            let _ = shell.set_attribute("data-crt-font", font_slug(st.font));
            let _ = shell.set_attribute("data-skin", skin_slug(st.skin));
            let _ = shell.set_attribute(
                "data-lazy-dyslexic",
                if st.lazy_dyslexic_loaded {
                    "true"
                } else {
                    "false"
                },
            );
            let _ = shell.set_attribute("data-area", area_slug(st.route.area));
            let _ = shell.set_attribute("data-subview", st.route.subview.slug());
            let (auth_slug, auth_href, auth_label) = match st.auth() {
                AuthState::Anonymous => (
                    "anonymous",
                    st.sign_in_href(),
                    "READ-ONLY · SIGN IN".to_owned(),
                ),
                AuthState::InviteRequired { github_login } => (
                    "invite_required",
                    st.sign_out_href(),
                    format!("INVITE REQ (@{github_login})"),
                ),
                AuthState::Operator { github_login, .. } => (
                    "operator",
                    st.sign_out_href(),
                    format!("@{github_login} · OPERATOR"),
                ),
            };
            let _ = shell.set_attribute("data-auth-state", auth_slug);
            if let Some(skin_btn) = doc.get_element_by_id("muconsole-skin-toggle") {
                skin_btn.set_text_content(Some("SKIN: WIREFRAME"));
            }
            if let Some(auth_link) = doc.get_element_by_id("muconsole-auth-link") {
                let _ = auth_link.set_attribute("href", &auth_href);
                let _ = auth_link.set_attribute("data-auth-state", auth_slug);
                auth_link.set_text_content(Some(&auth_label));
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
        LAST_SVG.with(|last| *last.borrow_mut() = svg.clone());
        let html = render_html_shell(&svg, st);
        body.set_inner_html(&html);
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

/// Spawn the worker and wire the message and DOM interaction bridges.
pub fn start() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let location = window.location();
    let raw_search = location.search().unwrap_or_default();
    let hash = location.hash().unwrap_or_default();
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
        st.restore_stored_repo(stored_repo.as_deref());
        sync_browser_state(&st);
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
                sync_browser_state(&st);
                repaint(&st);
                console::log_1(&JsValue::from_str(&st.client.summary()));
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
                sync_browser_state(&st);
                repaint(&st);
            }
        });
        let _ = window
            .add_event_listener_with_callback("hashchange", on_hashchange.as_ref().unchecked_ref());
        on_hashchange.forget();
    }

    // Document delegated click listener, keydown listener, wheel listener, and visibilitychange.
    if let Some(doc) = window.document() {
        {
            let state_for_click = Rc::clone(&state);
            let worker_for_click = worker.clone();
            let on_click = Closure::<dyn FnMut(MouseEvent)>::new(move |ev: MouseEvent| {
                let Some(target) = ev.target().and_then(|t| t.dyn_into::<Element>().ok()) else {
                    return;
                };
                if let Some((attr, val)) = find_action_attribute(&target) {
                    let mut st = state_for_click.borrow_mut();
                    let cmds = st.on_dom_action(attr, &val, ev.shift_key(), now_ms());
                    for cmd in &cmds {
                        if let UiCommand::Navigate { ref href } = cmd {
                            if let Some(win) = web_sys::window() {
                                let _ = win.location().set_href(href);
                            }
                        }
                    }
                    send_all(&worker_for_click, &cmds);
                    sync_browser_state(&st);
                    repaint(&st);
                }
            });
            let _ =
                doc.add_event_listener_with_callback("click", on_click.as_ref().unchecked_ref());
            on_click.forget();
        }

        {
            let state_for_key = Rc::clone(&state);
            let worker_for_key = worker.clone();
            let on_keydown = Closure::<dyn FnMut(KeyboardEvent)>::new(move |ev: KeyboardEvent| {
                let key = ev.key();
                let target_is_input = ev
                    .target()
                    .and_then(|t| t.dyn_into::<Element>().ok())
                    .is_some_and(|el| el.id() == "muconsole-filter");
                if target_is_input && key != "Escape" {
                    if key == "Enter" {
                        let mut st = state_for_key.borrow_mut();
                        st.pending_filter_at_ms = None;
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
                        .and_then(|d| d.get_element_by_id("muconsole-filter"))
                        .and_then(|e| e.dyn_into::<HtmlInputElement>().ok())
                    {
                        let _ = input.blur();
                    }
                }
                sync_browser_state(&st);
                repaint(&st);
            });
            let _ = doc
                .add_event_listener_with_callback("keydown", on_keydown.as_ref().unchecked_ref());
            on_keydown.forget();
        }

        {
            let state_for_input = Rc::clone(&state);
            let on_input = Closure::<dyn FnMut(Event)>::new(move |ev: Event| {
                let Some(input) = ev
                    .target()
                    .and_then(|t| t.dyn_into::<HtmlInputElement>().ok())
                else {
                    return;
                };
                if input.id() != "muconsole-filter" {
                    return;
                }
                let val = input.value();
                let mut st = state_for_input.borrow_mut();
                st.set_filter_query(&val, now_ms());
                sync_browser_state(&st);
                repaint(&st);
            });
            let _ =
                doc.add_event_listener_with_callback("input", on_input.as_ref().unchecked_ref());
            on_input.forget();
        }

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
                if in_top_bar {
                    if ev.shift_key() {
                        st.rotate_inner(delta, now_ms());
                    } else {
                        st.rotate_outer(delta, now_ms());
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
                sync_browser_state(&st);
                repaint(&st);
            });
            let _ =
                doc.add_event_listener_with_callback("wheel", on_wheel.as_ref().unchecked_ref());
            on_wheel.forget();
        }

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

    // Periodic 50ms tick for rotary settle debounce (`ROTARY_SETTLE_MS = 350`).
    {
        let state_for_tick = Rc::clone(&state);
        let worker_for_tick = worker.clone();
        let on_tick = Closure::<dyn FnMut()>::new(move || {
            let mut st = state_for_tick.borrow_mut();
            let cmds = st.tick(now_ms());
            if !cmds.is_empty() {
                send_all(&worker_for_tick, &cmds);
                sync_browser_state(&st);
                repaint(&st);
            }
        });
        let _ = window.set_interval_with_callback_and_timeout_and_arguments_0(
            on_tick.as_ref().unchecked_ref(),
            50,
        );
        on_tick.forget();
    }

    WORKER.with(|slot| *slot.borrow_mut() = Some(worker));
}
