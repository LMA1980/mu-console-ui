// SPDX-License-Identifier: AGPL-3.0-only
//! Rendering entry points and HTML chrome shell for [`ConsoleState`].
//!
//! Renders [`ConsoleScene`](mu_console_theme::scene::ConsoleScene) and
//! [`DialogModel`](mu_console_theme::scene::DialogModel) through either
//! [`HtmlCoreSkin`] (default) or [`WireframeSkin`] according to
//! [`ConsoleState::effective_skin`].

use crate::copy;
use crate::scene::{build_console_scene, build_dialog_model};
use crate::state::ConsoleState;
use mu_console_layout::{SafeArea, Viewport};
use mu_console_protocol::{
    Area, AuthStatus, ConsoleSkin, CrtFont, DialogModel as ProtoDialogModel, RiskConfirmAction,
    UserProfileTab,
};
use mu_console_theme::html::HtmlCoreSkin;
use mu_console_theme::scene::{SceneRenderer, SceneViewport};
use mu_console_theme::svg::escape_into;
use mu_console_theme::wireframe::WireframeSkin;
use std::fmt::Write as _;

/// Placeholder string for `#muconsole-filter` when viewing `area`.
#[must_use]
pub const fn filter_placeholder(area: Area) -> &'static str {
    copy::filter_placeholder_for_area(area)
}

/// Escape text safely for HTML and XML attributes/body nodes.
fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    escape_into(&mut out, s);
    out
}

/// Render the current [`ConsoleState`] using `state.effective_skin()`.
#[must_use]
pub fn render_console(state: &ConsoleState, vp: Viewport, _safe: SafeArea) -> String {
    render_console_with_skin(state.effective_skin(), state, vp)
}

/// Render [`ConsoleState`] using an explicit [`ConsoleSkin`].
#[must_use]
pub fn render_console_with_skin(skin: ConsoleSkin, state: &ConsoleState, vp: Viewport) -> String {
    let scene_vp = SceneViewport::new(vp.width, vp.height);
    let scene = build_console_scene(state, scene_vp);
    let dialog = build_dialog_model(state);
    match skin {
        ConsoleSkin::Wireframe => {
            WireframeSkin.render_scene_with_dialog(&scene, dialog.as_ref(), scene_vp)
        }
        ConsoleSkin::Html => {
            let html = HtmlCoreSkin.render_scene_with_dialog(&scene, dialog.as_ref(), scene_vp);
            enrich_html_dialog(html, state)
        }
    }
}

/// Enrich HTML5 `<dialog>` panels with interactive fields beyond the core
/// shared schema (3-tab `@user` editor and A|B rollback note input).
fn enrich_html_dialog(html: String, state: &ConsoleState) -> String {
    let Some(dlg) = state.active_dialog() else {
        return html;
    };
    if matches!(dlg, ProtoDialogModel::PasskeyEnrollmentGate(_)) {
        return html;
    }

    match dlg {
        ProtoDialogModel::UserProfile(_) => {
            let panel = render_user_profile_tabs_html(state);
            inject_before_dialog_close(html, &panel)
        }
        ProtoDialogModel::RiskConfirm(rc)
            if matches!(rc.action, RiskConfirmAction::TriggerAbRollback { .. }) =>
        {
            let note_val = escape_html(&state.platform_form.ab_rollback_note);
            let field = format!(
                r##"<div class="mu-field"><label for="muconsole-ab-rollback-note">Rollback reason (optional)</label><input id="muconsole-ab-rollback-note" class="mu-input" type="text" data-action="ab-rollback-note-input" placeholder="Optional incident note" value="{note_val}" style="font-size:16px"></div>"##
            );
            inject_before_dialog_close(html, &field)
        }
        _ => html,
    }
}

fn inject_before_dialog_close(html: String, fragment: &str) -> String {
    if let Some(idx) = html.rfind("</dialog>") {
        let mut out = String::with_capacity(html.len() + fragment.len());
        out.push_str(&html[..idx]);
        out.push_str(fragment);
        out.push_str(&html[idx..]);
        out
    } else {
        html
    }
}

fn render_user_profile_tabs_html(state: &ConsoleState) -> String {
    let Some(ProtoDialogModel::UserProfile(up)) = state.active_dialog() else {
        return String::new();
    };
    let tab = state.profile_tab;
    let is_details = tab == UserProfileTab::UserDetails;
    let is_prefs = tab == UserProfileTab::Preferences;
    let is_keys = tab == UserProfileTab::Passkeys;

    let mut out = String::from(
        r##"<div class="mu-profile-extra" data-component-id="muconsole-profile-extra">"##,
    );
    let _ = write!(
        out,
        r##"<div class="mu-segmented" role="tablist" aria-label="Profile sections"><button type="button" role="tab" class="mu-tab" data-control-id="profile-tab:details" data-action="profile-tab:details" aria-selected="{is_details}">{}</button><button type="button" role="tab" class="mu-tab" data-control-id="profile-tab:preferences" data-action="profile-tab:preferences" aria-selected="{is_prefs}">{}</button><button type="button" role="tab" class="mu-tab" data-control-id="profile-tab:passkeys" data-action="profile-tab:passkeys" aria-selected="{is_keys}">{}</button></div>"##,
        escape_html(copy::TAB_USER_DETAILS),
        escape_html(copy::TAB_PREFERENCES),
        escape_html(copy::TAB_PASSKEYS),
    );

    match tab {
        UserProfileTab::UserDetails => {
            let draft = if !state.profile_display_name_draft.is_empty() {
                state.profile_display_name_draft.as_str()
            } else {
                up.profile
                    .display_name
                    .as_deref()
                    .unwrap_or(&up.profile.github_login)
            };
            let _ = write!(
                out,
                r##"<div class="mu-field"><label for="muconsole-profile-display-name">Display Name</label><input id="muconsole-profile-display-name" class="mu-input" type="text" data-action="profile-display-name-input" value="{}" style="font-size:16px"><button type="button" class="mu-btn mu-btn-primary" data-control-id="profile-save-details" data-action="profile-save-details">Save Display Name</button></div>"##,
                escape_html(draft),
            );
        }
        UserProfileTab::Preferences => {
            let html_active = state.skin == ConsoleSkin::Html;
            let wf_active = state.skin == ConsoleSkin::Wireframe;
            let hurmit_active = state.font == CrtFont::Hurmit;
            let dys_active = state.font == CrtFont::OpenDyslexic;
            let _ = write!(
                out,
                r##"<div class="mu-pref-group"><span>Console Skin</span><div class="mu-btn-row"><button type="button" class="mu-btn" data-control-id="profile-skin:html" data-action="profile-skin:html" aria-pressed="{html_active}">{}</button><button type="button" class="mu-btn" data-control-id="profile-skin:wireframe" data-action="profile-skin:wireframe" aria-pressed="{wf_active}">{}</button></div></div><div class="mu-pref-group"><span>Typography</span><div class="mu-btn-row"><button type="button" class="mu-btn" data-control-id="profile-font:hurmit" data-action="profile-font:hurmit" aria-pressed="{hurmit_active}">{}</button><button type="button" class="mu-btn" data-control-id="profile-font:dyslexic" data-action="profile-font:dyslexic" aria-pressed="{dys_active}">{}</button></div></div>"##,
                escape_html(copy::SKIN_HTML_LABEL),
                escape_html(copy::SKIN_WIREFRAME_LABEL),
                escape_html(copy::FONT_HURMIT_LABEL),
                escape_html(copy::FONT_DYSLEXIC_LABEL),
            );
        }
        UserProfileTab::Passkeys => {
            let count = up.profile.passkey_count;
            let _ = write!(
                out,
                r##"<div class="mu-passkey-panel"><p>Enrolled passkeys: {count}</p><ul class="mu-kv">"##,
            );
            for pk in &up.profile.passkeys {
                let disable_remove = count <= 1;
                let disabled_attr = if disable_remove {
                    " disabled=\"disabled\" aria-disabled=\"true\""
                } else {
                    ""
                };
                let label = pk
                    .label
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .unwrap_or(&pk.credential_id);
                let _ = write!(
                    out,
                    r##"<li class="mu-kv-row"><span>{}</span><button type="button" class="mu-btn mu-btn-danger" data-control-id="profile-passkey-remove:{}" data-action="profile-passkey-remove:{}"{disabled_attr}>Remove</button></li>"##,
                    escape_html(label),
                    escape_html(&pk.credential_id),
                    escape_html(&pk.credential_id),
                );
            }
            let _ = write!(
                out,
                r##"</ul><button type="button" class="mu-btn mu-btn-primary" data-control-id="profile-passkey-add" data-action="profile-passkey-add">Add Passkey</button></div>"##
            );
        }
    }

    out.push_str("</div>");
    out
}

/// Wrap the rendered scene content (HTML or SVG) inside the semantic console
/// application shell (`#muconsole-shell`).
///
/// When `content` is pure SVG (`WireframeSkin`), all shell tags remain
/// XML-well-formed so XML DOM tests (`roxmltree::Document::parse`) validate
/// cleanly.
#[must_use]
pub fn render_html_shell(content: &str, state: &ConsoleState) -> String {
    let auth = state.auth();
    let skin_str = state.effective_skin().as_str();
    let font_css = match state.font {
        CrtFont::Hurmit => "var(--font-crt)",
        CrtFont::OpenDyslexic => "'OpenDyslexicM Nerd Font Mono',var(--font-crt)",
    };

    let placeholder = escape_html(filter_placeholder(state.route.area));
    let filter_val = escape_html(&state.filter_query);
    let polite = escape_html(&state.aria_polite);
    let assertive = escape_html(&state.aria_assertive);

    let mut styles = String::from(
        "#muconsole-shell{display:flex;flex-direction:column;}\
         #muconsole-filter{font-size:16px;}\
         .skip-link{position:absolute;left:-9999px;top:auto;}\
         .skip-link:focus{position:static;}",
    );
    if state.lazy_dyslexic_loaded {
        styles.push_str(
            "@font-face{font-family:'OpenDyslexicM Nerd Font Mono';font-style:normal;font-weight:400;font-display:swap;src:url('/fonts/OpenDyslexicMNerdFontMono-Regular.woff2') format('woff2');}",
        );
    }

    let font_toggle_btn = match state.font {
        CrtFont::Hurmit => format!("{} {}", copy::FONT_TOGGLE_SHORT, copy::FONT_HURMIT_LABEL),
        CrtFont::OpenDyslexic => {
            format!("{} {}", copy::FONT_TOGGLE_SHORT, copy::FONT_DYSLEXIC_LABEL)
        }
    };

    let header_tail = match auth.status {
        AuthStatus::Unresolved => {
            r##"<span id="muconsole-session-loading" data-auth-state="unresolved">Loading session…</span>"##.to_owned()
        }
        AuthStatus::Anonymous => {
            let href = escape_html(&state.sign_in_href());
            format!(
                r##"<button id="muconsole-font-toggle" type="button" data-control-id="c2-font-switch" data-action="c2-font-switch" aria-label="Toggle monospace font">{}</button><a id="muconsole-auth-link" href="{href}" data-auth-state="anonymous">{}</a>"##,
                escape_html(&font_toggle_btn),
                escape_html(copy::SIGN_IN_LABEL),
            )
        }
        AuthStatus::AuthorizationDenied => {
            let href = escape_html(&state.sign_out_href());
            format!(
                r##"<button id="muconsole-font-toggle" type="button" data-control-id="c2-font-switch" data-action="c2-font-switch" aria-label="Toggle monospace font">{}</button><span id="muconsole-auth-denied" data-auth-state="authorization_denied">{}</span><a id="muconsole-auth-link" href="{href}" data-auth-state="authorization_denied">{}</a>"##,
                escape_html(&font_toggle_btn),
                escape_html(copy::AUTH_DENIED_STATUS),
                escape_html(copy::SIGN_OUT_LABEL),
            )
        }
        AuthStatus::PasskeyEnrollmentRequired | AuthStatus::Active => {
            // Active sessions remove `[Aa]` from the top bar and surface `@user` + `+role[rem][idle]`.
            let gated_attr = if auth.passkey_enrollment_required
                || auth.status == AuthStatus::PasskeyEnrollmentRequired
            {
                " disabled=\"disabled\" aria-disabled=\"true\""
            } else {
                ""
            };
            let handle = escape_html(auth.github_login.as_deref().unwrap_or("operator"));
            let roles_text = escape_html(&state.format_header_roles());
            let href = escape_html(&state.sign_out_href());
            format!(
                r##"<button id="muconsole-user-btn" type="button" data-control-id="hdr-user" data-action="hdr-user"{gated_attr}>@{handle}</button><button id="muconsole-roles-btn" type="button" data-control-id="hdr-role" data-action="hdr-role"{gated_attr}>{roles_text}</button><a id="muconsole-auth-link" href="{href}" data-auth-state="active">{}</a>"##,
                escape_html(copy::SIGN_OUT_LABEL),
            )
        }
    };

    let skip_label = escape_html(copy::SKIP_TO_MAIN);
    let aria_label = escape_html(copy::FILTER_ARIA_LABEL);

    format!(
        r##"<div id="muconsole-shell" data-skin="{skin_str}" data-font="{font_css}" style="--font-active:{font_css}"><style>{styles}</style><header id="muconsole-header"><a class="skip-link" href="#muconsole-dashboard">{skip_label}</a><input id="muconsole-filter" type="search" value="{filter_val}" placeholder="{placeholder}" aria-label="{aria_label}" style="font-size:16px" /><div id="muconsole-aria-polite" role="status" aria-live="polite">{polite}</div><div id="muconsole-aria-assertive" role="alert" aria-live="assertive">{assertive}</div>{header_tail}</header><main id="muconsole-dashboard">{content}</main></div>"##
    )
}
