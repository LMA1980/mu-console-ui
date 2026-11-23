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
    RoleActivationKind, RoleLeaseView, UserProfileTab,
};
use mu_console_theme::html::HtmlCoreSkin;
use mu_console_theme::scene::{SceneRenderer, SceneViewport};
use mu_console_theme::svg::escape_into;
use mu_console_theme::wireframe::WireframeSkin;
use std::fmt::Write as _;

pub(crate) const DYSLEXIC_FONT_FACE_CSS: &str = concat!(
    "@font-face{font-family:'OpenDyslexicM Nerd Font Mono';font-style:normal;font-weight:400;font-display:swap;src:url('/fonts/OpenDyslexicMNerdFontMono-Regular.woff2') format('woff2');}",
    "@font-face{font-family:'OpenDyslexicM Nerd Font Mono';font-style:normal;font-weight:700;font-display:swap;src:url('/fonts/OpenDyslexicMNerdFontMono-Regular.woff2') format('woff2');}"
);

pub(crate) const fn font_stack_css(font: CrtFont) -> &'static str {
    match font {
        CrtFont::Hurmit => "'MuCRT',monospace",
        CrtFont::OpenDyslexic => "'OpenDyslexicM Nerd Font Mono','MuCRT',monospace",
    }
}

pub(crate) fn font_toggle_label(font: CrtFont) -> String {
    match font {
        CrtFont::Hurmit => format!("{} {}", copy::FONT_TOGGLE_SHORT, copy::FONT_HURMIT_LABEL),
        CrtFont::OpenDyslexic => {
            format!("{} {}", copy::FONT_TOGGLE_SHORT, copy::FONT_DYSLEXIC_LABEL)
        }
    }
}

/// Placeholder string for `#muconsole-filter` when viewing `area`.
#[must_use]
pub const fn filter_placeholder(area: Area) -> &'static str {
    copy::filter_placeholder_for_area(area)
}

/// Area-, subview-, and access-aware placeholder string for `#muconsole-filter`.
#[must_use]
pub fn filter_placeholder_for_state(state: &ConsoleState) -> &'static str {
    state.filter_placeholder_text()
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
            let svg = WireframeSkin.render_scene_with_dialog(&scene, dialog.as_ref(), scene_vp);
            let enriched = enrich_wireframe_dialog(svg, state, scene_vp);
            enrich_wireframe_lease_timers(enriched, state)
        }
        ConsoleSkin::Html => {
            let html = HtmlCoreSkin.render_scene_with_dialog(&scene, dialog.as_ref(), scene_vp);
            let enriched = enrich_html_dialog(html, state);
            enrich_html_lease_timers(enriched, state)
        }
    }
}

fn strip_bracketed_mm_ss(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(open) = rest.find('[') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        if let Some(close) = after.find(']') {
            let inner = &after[..close];
            if !inner.is_empty()
                && inner.contains(':')
                && inner.bytes().all(|b| b.is_ascii_digit() || b == b':')
            {
                out.push_str("[__T__]");
                rest = &after[close + 1..];
                continue;
            }
        }
        out.push('[');
        rest = after;
    }
    out.push_str(rest);
    out
}

/// Normalize dynamic `[data-lease-timer]` text nodes and `hdr-role` `aria-label` timer strings
/// so structural equality checks skip DOM replacement when only countdown digits ticked.
#[must_use]
pub fn strip_lease_timer_values(markup: &str) -> String {
    let mut out = markup.to_owned();

    for (open_pat, close_tag) in [
        (
            r#"<span class="mu-lease-timer" data-lease-timer=""#,
            "</span>",
        ),
        (
            r#"<tspan class="mu-lease-timer" data-lease-timer=""#,
            "</tspan>",
        ),
    ] {
        let mut cursor = 0;
        while let Some(rel) = out[cursor..].find(open_pat) {
            let start = cursor + rel;
            let Some(rel_gt) = out[start..].find('>') else {
                break;
            };
            let body_start = start + rel_gt + 1;
            let Some(rel_end) = out[body_start..].find(close_tag) else {
                break;
            };
            let body_end = body_start + rel_end;
            out.replace_range(body_start..body_end, "__TIMER__");
            cursor = body_start + "__TIMER__".len() + close_tag.len();
        }
    }

    let mut cursor = 0;
    while let Some(rel) = out[cursor..].find(r#"data-action="hdr-role""#) {
        let marker = cursor + rel;
        let Some(tag_start) = out[..marker].rfind('<') else {
            cursor = marker + 1;
            continue;
        };
        let Some(rel_tag_end) = out[marker..].find('>') else {
            break;
        };
        let tag_end = marker + rel_tag_end;
        if let Some(rel_aria) = out[tag_start..tag_end].find(r#"aria-label=""#) {
            let val_start = tag_start + rel_aria + r#"aria-label=""#.len();
            if let Some(rel_quote) = out[val_start..tag_end].find('"') {
                let val_end = val_start + rel_quote;
                let normalized = strip_bracketed_mm_ss(&out[val_start..val_end]);
                out.replace_range(val_start..val_end, &normalized);
                cursor = val_start + normalized.len() + 1;
                continue;
            }
        }
        cursor = tag_end + 1;
    }

    out
}

/// Wrap HTML lease timer numerals in `[data-lease-timer]` spans so background countdown
/// ticks update only timer text nodes without redrawing banners, buttons, or modals.
#[must_use]
pub fn enrich_html_lease_timers(mut html: String, state: &ConsoleState) -> String {
    let auth = state.auth();
    if auth.leases.is_empty() {
        return html;
    }

    let roles_plain_esc = escape_html(&state.format_header_roles());
    let roles_html = state.format_header_roles_html();
    if roles_html != roles_plain_esc {
        let mut cursor = 0;
        while let Some(rel) = html[cursor..].find(r#"data-action="hdr-role""#) {
            let marker = cursor + rel;
            let Some(rel_gt) = html[marker..].find('>') else {
                break;
            };
            let body_start = marker + rel_gt + 1;
            let Some(rel_end) = html[body_start..].find("</button>") else {
                break;
            };
            let body_end = body_start + rel_end;
            if html[body_start..body_end] == roles_plain_esc {
                html.replace_range(body_start..body_end, &roles_html);
                break;
            }
            cursor = body_end;
        }
    }

    for lease in &auth.leases {
        if !(lease.can_renew && !lease.is_expired()) {
            continue;
        }
        let id_esc = escape_html(&lease.role_id);
        let rem = RoleLeaseView::format_mm_ss(lease.seconds_remaining);
        let idle = RoleLeaseView::format_mm_ss(lease.idle_timer);
        let banner_attr = format!(r#"data-banner-id="banner-role-renew-{id_esc}""#);
        if let Some(b_pos) = html.find(&banner_attr) {
            let next_banner = html[b_pos + 1..]
                .find(r#"data-banner-id=""#)
                .map_or(html.len(), |n| b_pos + 1 + n);
            let grid_pos = html[b_pos..]
                .find(r#"class="mu-scene-grid""#)
                .map_or(html.len(), |n| b_pos + n);
            let b_limit = next_banner.min(grid_pos);
            if let Some(rel_p_start) = html[b_pos..b_limit].find("<p>") {
                let p_start = b_pos + rel_p_start;
                if let Some(rel_p_end) = html[p_start..b_limit].find("</p>") {
                    let p_end = p_start + rel_p_end;
                    let mut seg = html[p_start..p_end].to_owned();
                    let rem_from = format!("expires in {rem}");
                    let rem_to = format!(
                        r#"expires in <span class="mu-lease-timer" data-lease-timer="renew-rem:{id_esc}">{rem}</span>"#
                    );
                    if let Some(idx) = seg.find(&rem_from) {
                        seg.replace_range(idx..idx + rem_from.len(), &rem_to);
                    }
                    let idle_from = format!("(idle {idle})");
                    let idle_to = format!(
                        r#"(idle <span class="mu-lease-timer" data-lease-timer="renew-idle:{id_esc}">{idle}</span>)"#
                    );
                    if let Some(idx) = seg.find(&idle_from) {
                        seg.replace_range(idx..idx + idle_from.len(), &idle_to);
                    }
                    html.replace_range(p_start..p_end, &seg);
                }
            }
        }
    }

    if let Some(dlg_start) = html.find(r#"id="dlg-session-roles""#) {
        let dlg_end = html[dlg_start..]
            .find("</dialog>")
            .map_or(html.len(), |n| dlg_start + n);
        let mut dlg_seg = html[dlg_start..dlg_end].to_owned();
        for lease in &auth.leases {
            let id_esc = escape_html(&lease.role_id);
            let rem = RoleLeaseView::format_mm_ss(lease.seconds_remaining);
            let old_exp = format!(r#"<span class="mu-role-expiry">Expires {rem}</span>"#);
            let new_exp = format!(
                r#"<span class="mu-role-expiry">Expires <span class="mu-lease-timer" data-lease-timer="dlg-rem:{id_esc}">{rem}</span></span>"#
            );
            let disable_marker = format!(r#"data-control-id="role-disable:{id_esc}""#);
            if let Some(btn_pos) = dlg_seg.find(&disable_marker) {
                let li_start = dlg_seg[..btn_pos].rfind("<li ").unwrap_or(0);
                let li_end = dlg_seg[btn_pos..]
                    .find("</li>")
                    .map_or(dlg_seg.len(), |n| btn_pos + n);
                if let Some(rel) = dlg_seg[li_start..li_end].find(&old_exp) {
                    let abs = li_start + rel;
                    dlg_seg.replace_range(abs..abs + old_exp.len(), &new_exp);
                }
            }
            let renew_marker = format!(r#"data-control-id="role-renew:{id_esc}""#);
            if let Some(btn_pos) = dlg_seg.find(&renew_marker) {
                let li_start = dlg_seg[..btn_pos].rfind("<li ").unwrap_or(0);
                let li_end = dlg_seg[btn_pos..]
                    .find("</li>")
                    .map_or(dlg_seg.len(), |n| btn_pos + n);
                let old_rem = format!("({rem} remaining)");
                let new_rem = format!(
                    r#"(<span class="mu-lease-timer" data-lease-timer="dlg-rem:{id_esc}">{rem}</span> remaining)"#
                );
                if let Some(rel) = dlg_seg[li_start..li_end].find(&old_rem) {
                    let abs = li_start + rel;
                    dlg_seg.replace_range(abs..abs + old_rem.len(), &new_rem);
                }
                let li_end = dlg_seg[btn_pos..]
                    .find("</li>")
                    .map_or(dlg_seg.len(), |n| btn_pos + n);
                if let Some(rel) = dlg_seg[li_start..li_end].find(&old_exp) {
                    let abs = li_start + rel;
                    dlg_seg.replace_range(abs..abs + old_exp.len(), &new_exp);
                }
            }
        }
        html.replace_range(dlg_start..dlg_end, &dlg_seg);
    }

    if let Some(dlg_start) = html.find(r#"id="dlg-user-profile""#) {
        let dlg_end = html[dlg_start..]
            .find("</dialog>")
            .map_or(html.len(), |n| dlg_start + n);
        if let Some(rel_grp) = html[dlg_start..dlg_end].find(r#"data-role-state="active""#) {
            let grp_start = dlg_start + rel_grp;
            let grp_end = html[grp_start..dlg_end]
                .find("</ul>")
                .map_or(dlg_end, |n| grp_start + n);
            let mut grp_seg = html[grp_start..grp_end].to_owned();
            for badge in &auth.active_roles {
                if !(matches!(badge.activation.kind, RoleActivationKind::Timed)
                    || badge.lease.is_some())
                {
                    continue;
                }
                let Some(lease) = badge
                    .lease
                    .as_ref()
                    .or_else(|| auth.leases.iter().find(|l| l.role_id == badge.role_id))
                else {
                    continue;
                };
                let id_esc = escape_html(&badge.role_id);
                let role_esc = escape_html(badge.effective_label());
                let secs = format!("{}s", lease.seconds_remaining);
                let old = format!(">{role_esc} ({secs} remaining)</li>");
                let new = format!(
                    r#">{role_esc} (<span class="mu-lease-timer" data-lease-timer="profile-secs:{id_esc}">{secs}</span> remaining)</li>"#
                );
                if let Some(idx) = grp_seg.find(&old) {
                    grp_seg.replace_range(idx..idx + old.len(), &new);
                }
            }
            html.replace_range(grp_start..grp_end, &grp_seg);
        }
    }

    html
}

/// Wrap Wireframe SVG lease timer numerals in `<tspan class="mu-lease-timer" data-lease-timer="...">`
/// so background countdown ticks update only the SVG text spans in-place.
#[must_use]
pub fn enrich_wireframe_lease_timers(mut svg: String, state: &ConsoleState) -> String {
    let auth = state.auth();
    if auth.leases.is_empty() {
        return svg;
    }

    let roles_plain_esc = escape_html(&state.format_header_roles());
    let aria_needle = format!(r#"aria-label="{roles_plain_esc}""#);
    let mut cursor = 0;
    while let Some(rel) = svg[cursor..].find(r#"data-action="hdr-role""#) {
        let marker = cursor + rel;
        let Some(g_start) = svg[..marker].rfind("<g") else {
            cursor = marker + 1;
            continue;
        };
        let Some(rel_g_end) = svg[marker..].find("</g>") else {
            break;
        };
        let g_end = marker + rel_g_end;
        if svg[g_start..g_end].contains(&aria_needle) {
            if let Some(rel_text) = svg[g_start..g_end].find("<text ") {
                let t_open = g_start + rel_text;
                if let Some(rel_gt) = svg[t_open..g_end].find('>') {
                    let body_start = t_open + rel_gt + 1;
                    if let Some(rel_end) = svg[body_start..g_end].find("</text>") {
                        let body_end = body_start + rel_end;
                        let mut body = svg[body_start..body_end].to_owned();
                        let mut search_pos = 0;
                        for badge in &auth.active_roles {
                            if !(matches!(badge.activation.kind, RoleActivationKind::Timed)
                                || badge.lease.is_some())
                            {
                                continue;
                            }
                            let Some(lease) = &badge.lease else {
                                continue;
                            };
                            let label_esc = escape_html(&format!("+{}", badge.effective_label()));
                            let id_esc = escape_html(&badge.role_id);
                            let timer_full = format!(
                                "[{}][{}]",
                                RoleLeaseView::format_mm_ss(lease.seconds_remaining),
                                RoleLeaseView::format_mm_ss(lease.idle_timer)
                            );
                            if let Some(rel_idx) = body[search_pos..].find(&label_esc) {
                                let after_idx = search_pos + rel_idx + label_esc.len();
                                let after = &body[after_idx..];
                                if after.starts_with(&timer_full) {
                                    let repl = format!(
                                        r#"<tspan class="mu-lease-timer" data-lease-timer="hdr:{id_esc}">{timer_full}</tspan>"#
                                    );
                                    body.replace_range(
                                        after_idx..after_idx + timer_full.len(),
                                        &repl,
                                    );
                                    search_pos = after_idx + repl.len();
                                } else if after.starts_with('[') {
                                    let slice = after
                                        .split('<')
                                        .next()
                                        .unwrap_or("")
                                        .split('…')
                                        .next()
                                        .unwrap_or("");
                                    if !slice.is_empty() && timer_full.starts_with(slice) {
                                        let avail = slice.chars().count();
                                        let repl = format!(
                                            r#"<tspan class="mu-lease-timer" data-lease-timer="hdr-trunc:{id_esc}:{avail}">{slice}</tspan>"#
                                        );
                                        body.replace_range(
                                            after_idx..after_idx + slice.len(),
                                            &repl,
                                        );
                                        search_pos = after_idx + repl.len();
                                    }
                                }
                            }
                        }
                        svg.replace_range(body_start..body_end, &body);
                        break;
                    }
                }
            }
        }
        cursor = g_end + "</g>".len();
    }

    for lease in &auth.leases {
        if !(lease.can_renew && !lease.is_expired()) {
            continue;
        }
        let id_esc = escape_html(&lease.role_id);
        let label_esc = escape_html(lease.display_name.as_deref().unwrap_or(&lease.role_id));
        let rem = RoleLeaseView::format_mm_ss(lease.seconds_remaining);
        let idle = RoleLeaseView::format_mm_ss(lease.idle_timer);
        let marker = format!(
            ">{}: {label_esc} expires in ",
            escape_html(copy::RENEW_BANNER_TITLE)
        );
        if let Some(m_pos) = svg.find(&marker) {
            let text_start = m_pos + 1;
            if let Some(rel_end) = svg[text_start..].find("</text>") {
                let text_end = text_start + rel_end;
                let mut t = svg[text_start..text_end].to_owned();
                let rem_from = format!("expires in {rem}");
                let rem_to = format!(
                    r#"expires in <tspan class="mu-lease-timer" data-lease-timer="renew-rem:{id_esc}">{rem}</tspan>"#
                );
                if let Some(idx) = t.find(&rem_from) {
                    t.replace_range(idx..idx + rem_from.len(), &rem_to);
                }
                let idle_from = format!("(idle {idle})");
                let idle_to = format!(
                    r#"(idle <tspan class="mu-lease-timer" data-lease-timer="renew-idle:{id_esc}">{idle}</tspan>)"#
                );
                if let Some(idx) = t.find(&idle_from) {
                    t.replace_range(idx..idx + idle_from.len(), &idle_to);
                }
                svg.replace_range(text_start..text_end, &t);
            }
        }
    }

    if let Some(dlg_start) = svg.find(r#"id="dlg-session-roles""#) {
        let mut dlg_seg = svg[dlg_start..].to_owned();
        let mut search_pos = 0;
        for lease in &auth.leases {
            if !(lease.can_renew && !lease.is_expired()) {
                continue;
            }
            let id_esc = escape_html(&lease.role_id);
            let label_esc = escape_html(lease.display_name.as_deref().unwrap_or(&lease.role_id));
            let rem = RoleLeaseView::format_mm_ss(lease.seconds_remaining);
            let row_hint = format!("{label_esc} — Renew");
            let marker = "renewal window (";
            let scope_from = dlg_seg[search_pos..]
                .find(&row_hint)
                .map_or(search_pos, |idx| search_pos + idx);
            if let Some(m_rel) = dlg_seg[scope_from..].find(marker) {
                let val_start = scope_from + m_rel + marker.len();
                let after = &dlg_seg[val_start..];
                if after.starts_with(&rem) {
                    let new = format!(
                        r#"<tspan class="mu-lease-timer" data-lease-timer="dlg-rem:{id_esc}">{rem}</tspan>"#
                    );
                    dlg_seg.replace_range(val_start..val_start + rem.len(), &new);
                    search_pos = val_start + new.len();
                } else {
                    let slice = after
                        .split('<')
                        .next()
                        .unwrap_or("")
                        .split('…')
                        .next()
                        .unwrap_or("");
                    if !slice.is_empty() && rem.starts_with(slice) {
                        let avail = slice.chars().count();
                        let new = format!(
                            r#"<tspan class="mu-lease-timer" data-lease-timer="dlg-rem-trunc:{id_esc}:{avail}">{slice}</tspan>"#
                        );
                        dlg_seg.replace_range(val_start..val_start + slice.len(), &new);
                        search_pos = val_start + new.len();
                    }
                }
            }
        }
        svg.replace_range(dlg_start.., &dlg_seg);
    }

    svg
}

/// Enrich HTML5 `<dialog>` panels with interactive fields beyond the core
/// shared schema (3-tab `@user` editor and A|B rollback note input).
fn enrich_html_dialog(html: String, state: &ConsoleState) -> String {
    let Some(dlg) = state.active_dialog() else {
        return html;
    };
    if matches!(dlg, ProtoDialogModel::PasskeyEnrollmentGate(_)) {
        let mut out = html;
        let steps = r##"<ol class="mu-passkey-steps"><li>Select <strong>Register Passkey</strong> below to launch your passkey prompt.</li><li>Approve in 1Password, your browser or OS passkey manager, or touch a FIDO2 security key.</li><li>Select <strong>Not Now</strong> (or press Escape) to browse public Dashboard and CI views first, then enroll anytime via <strong>@user -&gt; Passkeys</strong>.</li></ol>"##;
        if let Some(idx) = out.find("<div class=\"mu-dialog-footer\">") {
            out.insert_str(idx, steps);
        }
        if !out.contains("dlg-passkey-dismiss") {
            if let Some(footer_idx) = out.find("<div class=\"mu-dialog-footer\">") {
                if let Some(rel_end) = out[footer_idx..].find("</button>") {
                    let insert_at = footer_idx + rel_end + "</button>".len();
                    out.insert_str(
                        insert_at,
                        r##"<button type="button" class="mu-btn" data-control-id="dlg-passkey-dismiss" data-action="dlg-passkey-dismiss">Not Now</button>"##,
                    );
                }
            }
        }
        return out;
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

fn find_svg_dialog_close_idx(svg: &str, dialog_id: &str) -> Option<usize> {
    let id_marker = format!("id=\"{dialog_id}\"");
    let id_pos = svg.find(&id_marker)?;
    let open_start = svg[..id_pos].rfind("<g")?;
    let mut depth = 0_usize;
    let mut i = open_start;
    while i < svg.len() {
        let rest = &svg[i..];
        if rest.starts_with("<g")
            && rest[2..]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_whitespace() || c == '>')
        {
            depth += 1;
            i += 2;
        } else if rest.starts_with("</g>") {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some(i);
            }
            i += 4;
        } else {
            let step = rest.chars().next().map_or(1, char::len_utf8);
            i += step;
        }
    }
    None
}

fn wf_fit_label(s: &str, width_px: f64, font_px: f64) -> String {
    let char_w = (font_px * 0.62).max(1.0);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let max_chars = (((width_px - 8.0).max(0.0) / char_w).floor() as usize).max(1);
    let count = s.chars().count();
    if count <= max_chars {
        return s.to_owned();
    }
    if max_chars == 1 {
        return "…".to_owned();
    }
    let mut out: String = s.chars().take(max_chars - 1).collect();
    out.push('…');
    out
}

#[allow(clippy::too_many_arguments)]
fn write_wf_profile_button(
    out: &mut String,
    control_id: &str,
    role: &str,
    label: &str,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    active: bool,
    disabled: bool,
) {
    let (fill, stroke, stroke_w, text_fill, font_weight) = if disabled {
        ("#0c1016", "#252d38", "1", "#636e7b", "normal")
    } else if active {
        ("#1f3452", "#00f0ff", "1.5", "#00f0ff", "bold")
    } else {
        ("#1b2230", "#2b3440", "1", "#e6edf3", "normal")
    };
    let state_attr = if role == "tab" {
        format!("aria-selected=\"{active}\" data-selected=\"{active}\"")
    } else {
        format!("aria-pressed=\"{active}\" data-selected=\"{active}\"")
    };
    let disabled_attrs = if disabled {
        r##" disabled="disabled" aria-disabled="true""##
    } else {
        r##" tabindex="0""##
    };
    let glyph = if active { "●" } else { "○" };
    let display = wf_fit_label(&format!("{glyph} {label}"), w, 9.5);
    let cid = escape_html(control_id);
    let aria = escape_html(label);
    let text_esc = escape_html(&display);
    let tx = x + 6.0;
    let ty = y + h * 0.5 + 3.5;

    let _ = write!(
        out,
        r##"<g role="{role}" data-control-id="{cid}" data-action="{cid}" {state_attr} aria-label="{aria}"{disabled_attrs} data-bbox-x="{x:.2}" data-bbox-y="{y:.2}" data-bbox-w="{w:.2}" data-bbox-h="{h:.2}"><rect x="{x:.2}" y="{y:.2}" width="{w:.2}" height="{h:.2}" rx="4" fill="{fill}" stroke="{stroke}" stroke-width="{stroke_w}"/><text x="{tx:.2}" y="{ty:.2}" fill="{text_fill}" font-family="MuCRT, monospace" font-size="9.5" font-weight="{font_weight}">{text_esc}</text></g>"##
    );
}

fn enrich_wireframe_dialog(svg: String, state: &ConsoleState, vp: SceneViewport) -> String {
    let Some(ProtoDialogModel::UserProfile(up)) = state.active_dialog() else {
        return svg;
    };
    let Some(insert_at) = find_svg_dialog_close_idx(&svg, "dlg-user-profile") else {
        return svg;
    };

    let margin = 12.0_f64;
    let max_w = (vp.width - margin * 2.0).max(20.0);
    let max_h = (vp.height - margin * 2.0).max(20.0);
    let card_w = max_w.min(540.0);
    let card_h = max_h.min(340.0);
    let card_x = ((vp.width - card_w) * 0.5).clamp(0.0, vp.width);
    let card_y = ((vp.height - card_h) * 0.5).clamp(0.0, vp.height);
    let card_w = card_w.clamp(0.0, (vp.width - card_x).max(0.0));
    let card_h = card_h.clamp(0.0, (vp.height - card_y).max(0.0));
    let footer_y = (card_y + card_h - 34.0).max(card_y);
    let body_top = card_y + 78.0;
    if footer_y - body_top < 130.0 || card_w < 160.0 {
        return svg;
    }

    let body_x = card_x + 12.0;
    let body_w = (card_w - 24.0).max(24.0);
    let tab_y = body_top;
    let tab_h = 22.0;
    let tab_gap = 6.0;
    let tab_w = ((body_w - tab_gap * 2.0) / 3.0).max(16.0);

    let mut frag = String::from(r##"<g id="muconsole-wf-profile-extra">"##);
    let tabs = [
        (
            "profile-tab:details",
            copy::TAB_USER_DETAILS,
            state.profile_tab == UserProfileTab::UserDetails,
        ),
        (
            "profile-tab:preferences",
            copy::TAB_PREFERENCES,
            state.profile_tab == UserProfileTab::Preferences,
        ),
        (
            "profile-tab:passkeys",
            copy::TAB_PASSKEYS,
            state.profile_tab == UserProfileTab::Passkeys,
        ),
    ];
    for (idx, (cid, label, selected)) in tabs.into_iter().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let tx = body_x + (idx as f64) * (tab_w + tab_gap);
        let tw = (body_x + body_w - tx).min(tab_w).max(0.0);
        write_wf_profile_button(
            &mut frag, cid, "tab", label, tx, tab_y, tw, tab_h, selected, false,
        );
    }

    let content_y = tab_y + tab_h + 12.0;
    let eff_skin = state.effective_skin();
    let active_skin_label = match eff_skin {
        ConsoleSkin::Html => copy::SKIN_HTML_LABEL,
        ConsoleSkin::Wireframe => copy::SKIN_WIREFRAME_LABEL,
    };
    let active_font_label = match state.font {
        CrtFont::Hurmit => copy::FONT_HURMIT_LABEL,
        CrtFont::OpenDyslexic => copy::FONT_DYSLEXIC_LABEL,
    };

    match state.profile_tab {
        UserProfileTab::UserDetails => {
            let name = if !state.profile_display_name_draft.is_empty() {
                state.profile_display_name_draft.as_str()
            } else {
                up.profile
                    .display_name
                    .as_deref()
                    .unwrap_or(&up.profile.github_login)
            };
            let line1 = wf_fit_label(&format!("Display Name: {name}"), body_w, 10.0);
            let line2 = wf_fit_label(
                &format!("Active: {active_skin_label} | {active_font_label}"),
                body_w,
                9.5,
            );
            let _ = write!(
                frag,
                r##"<text x="{bx:.2}" y="{y1:.2}" fill="#e6edf3" font-family="MuCRT, monospace" font-size="10.0">{l1}</text><text x="{bx:.2}" y="{y2:.2}" fill="#8b949e" font-family="MuCRT, monospace" font-size="9.5">{l2}</text>"##,
                bx = body_x,
                y1 = content_y + 14.0,
                l1 = escape_html(&line1),
                y2 = content_y + 34.0,
                l2 = escape_html(&line2),
            );
        }
        UserProfileTab::Preferences => {
            let html_active = eff_skin == ConsoleSkin::Html;
            let wf_active = eff_skin == ConsoleSkin::Wireframe;
            let wf_disabled = state.auth().status != AuthStatus::Active;
            let hurmit_active = state.font == CrtFont::Hurmit;
            let dys_active = state.font == CrtFont::OpenDyslexic;

            let val_x = (body_x + 96.0).min(body_x + body_w * 0.52);
            let val_w = (body_x + body_w - val_x).max(8.0);
            let skin_badge = wf_fit_label(&format!("Active: {active_skin_label}"), val_w, 9.5);
            let font_badge = wf_fit_label(&format!("Active: {active_font_label}"), val_w, 9.5);

            let btn_gap = 8.0;
            let btn_w = ((body_w - btn_gap) * 0.5).max(20.0);
            let btn_h = 22.0;

            let _ = write!(
                frag,
                r##"<text x="{bx:.2}" y="{hy1:.2}" fill="#e6edf3" font-family="MuCRT, monospace" font-size="10.0">Console Skin</text><text x="{vx:.2}" y="{hy1:.2}" fill="#00f0ff" font-family="MuCRT, monospace" font-size="9.5" font-weight="bold">{sb}</text>"##,
                bx = body_x,
                hy1 = content_y + 11.0,
                vx = val_x,
                sb = escape_html(&skin_badge),
            );
            let skin_y = content_y + 18.0;
            write_wf_profile_button(
                &mut frag,
                "profile-skin:html",
                "button",
                copy::SKIN_HTML_LABEL,
                body_x,
                skin_y,
                btn_w,
                btn_h,
                html_active,
                false,
            );
            let x2 = body_x + btn_w + btn_gap;
            let w2 = (body_x + body_w - x2).min(btn_w).max(0.0);
            write_wf_profile_button(
                &mut frag,
                "profile-skin:wireframe",
                "button",
                copy::SKIN_WIREFRAME_LABEL,
                x2,
                skin_y,
                w2,
                btn_h,
                wf_active,
                wf_disabled,
            );

            let font_hdr_y = skin_y + btn_h + 18.0;
            let _ = write!(
                frag,
                r##"<text x="{bx:.2}" y="{hy2:.2}" fill="#e6edf3" font-family="MuCRT, monospace" font-size="10.0">Typography</text><text x="{vx:.2}" y="{hy2:.2}" fill="#00f0ff" font-family="MuCRT, monospace" font-size="9.5" font-weight="bold">{fb}</text>"##,
                bx = body_x,
                hy2 = font_hdr_y,
                vx = val_x,
                fb = escape_html(&font_badge),
            );
            let font_y = skin_y + btn_h + 25.0;
            write_wf_profile_button(
                &mut frag,
                "profile-font:hurmit",
                "button",
                copy::FONT_HURMIT_LABEL,
                body_x,
                font_y,
                btn_w,
                btn_h,
                hurmit_active,
                false,
            );
            write_wf_profile_button(
                &mut frag,
                "profile-font:dyslexic",
                "button",
                copy::FONT_DYSLEXIC_LABEL,
                x2,
                font_y,
                w2,
                btn_h,
                dys_active,
                false,
            );
        }
        UserProfileTab::Passkeys => {
            let count = up.profile.passkey_count.max(state.auth().passkey_count);
            let line = wf_fit_label(&format!("Enrolled passkeys: {count}"), body_w, 10.0);
            let _ = write!(
                frag,
                r##"<text x="{bx:.2}" y="{y1:.2}" fill="#e6edf3" font-family="MuCRT, monospace" font-size="10.0">{line}</text>"##,
                bx = body_x,
                y1 = content_y + 14.0,
                line = escape_html(&line),
            );
            write_wf_profile_button(
                &mut frag,
                "profile-passkey-add",
                "button",
                "Add Passkey",
                body_x,
                content_y + 24.0,
                body_w.min(160.0),
                22.0,
                false,
                false,
            );
        }
    }

    frag.push_str("</g>");
    let mut out = String::with_capacity(svg.len() + frag.len());
    out.push_str(&svg[..insert_at]);
    out.push_str(&frag);
    out.push_str(&svg[insert_at..]);
    out
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
            let eff_skin = state.effective_skin();
            let html_active = eff_skin == ConsoleSkin::Html;
            let wf_active = eff_skin == ConsoleSkin::Wireframe;
            let wf_disabled = state.auth().status != AuthStatus::Active;
            let wf_disabled_attr = if wf_disabled {
                " disabled=\"disabled\" aria-disabled=\"true\""
            } else {
                ""
            };
            let active_skin_label = match eff_skin {
                ConsoleSkin::Html => copy::SKIN_HTML_LABEL,
                ConsoleSkin::Wireframe => copy::SKIN_WIREFRAME_LABEL,
            };
            let hurmit_active = state.font == CrtFont::Hurmit;
            let dys_active = state.font == CrtFont::OpenDyslexic;
            let active_font_label = match state.font {
                CrtFont::Hurmit => copy::FONT_HURMIT_LABEL,
                CrtFont::OpenDyslexic => copy::FONT_DYSLEXIC_LABEL,
            };
            let html_glyph = if html_active { "●" } else { "○" };
            let wf_glyph = if wf_active { "●" } else { "○" };
            let hurmit_glyph = if hurmit_active { "●" } else { "○" };
            let dys_glyph = if dys_active { "●" } else { "○" };
            let _ = write!(
                out,
                r##"<div class="mu-pref-group"><div class="mu-pref-header"><span>Console Skin</span><span class="mu-pref-current" data-active-skin="{}">Active: {}</span></div><div class="mu-btn-row" role="group" aria-label="Console Skin"><button type="button" class="mu-btn" data-control-id="profile-skin:html" data-action="profile-skin:html" aria-pressed="{html_active}" data-selected="{html_active}"><span class="mu-signal" aria-hidden="true">{html_glyph}</span>{}</button><button type="button" class="mu-btn" data-control-id="profile-skin:wireframe" data-action="profile-skin:wireframe" aria-pressed="{wf_active}" data-selected="{wf_active}"{wf_disabled_attr}><span class="mu-signal" aria-hidden="true">{wf_glyph}</span>{}</button></div></div><div class="mu-pref-group"><div class="mu-pref-header"><span>Typography</span><span class="mu-pref-current" data-active-font="{}">Active: {}</span></div><div class="mu-btn-row" role="group" aria-label="Typography"><button type="button" class="mu-btn" data-control-id="profile-font:hurmit" data-action="profile-font:hurmit" aria-pressed="{hurmit_active}" data-selected="{hurmit_active}"><span class="mu-signal" aria-hidden="true">{hurmit_glyph}</span>{}</button><button type="button" class="mu-btn" data-control-id="profile-font:dyslexic" data-action="profile-font:dyslexic" aria-pressed="{dys_active}" data-selected="{dys_active}"><span class="mu-signal" aria-hidden="true">{dys_glyph}</span>{}</button></div></div>"##,
                eff_skin.as_str(),
                escape_html(active_skin_label),
                escape_html(copy::SKIN_HTML_LABEL),
                escape_html(copy::SKIN_WIREFRAME_LABEL),
                crate::state::font_slug(state.font),
                escape_html(active_font_label),
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
    let crt_font_slug = crate::state::font_slug(state.font);
    let font_css = font_stack_css(state.font);

    let placeholder = escape_html(filter_placeholder_for_state(state));
    let filter_disabled_attr = if state.is_filter_applicable() {
        ""
    } else {
        r#" disabled="disabled" aria-disabled="true""#
    };
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
        styles.push_str(DYSLEXIC_FONT_FACE_CSS);
    }

    let font_toggle_btn = font_toggle_label(state.font);

    let header_tail = match auth.status {
        AuthStatus::Unresolved => {
            if state.session_probe_error().is_some() {
                let href = escape_html(&state.sign_out_href());
                format!(
                    r##"<span id="muconsole-session-error" data-auth-state="unresolved">{}</span><a id="muconsole-auth-link" href="{href}" data-auth-state="unresolved">{}</a>"##,
                    escape_html(copy::SESSION_PROBE_FAILED_TITLE),
                    escape_html(copy::SIGN_OUT_LABEL),
                )
            } else {
                r##"<span id="muconsole-session-loading" data-auth-state="unresolved">Loading session…</span>"##.to_owned()
            }
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
            // Keep `@user` enabled during 0-Passkey onboarding so Passkey setup stays reachable.
            let role_gated_attr = if auth.passkey_enrollment_required
                || auth.status == AuthStatus::PasskeyEnrollmentRequired
            {
                " disabled=\"disabled\" aria-disabled=\"true\""
            } else {
                ""
            };
            let handle = escape_html(auth.github_login.as_deref().unwrap_or("operator"));
            let roles_plain = escape_html(&state.format_header_roles());
            let roles_html = state.format_header_roles_html();
            let href = escape_html(&state.sign_out_href());
            let auth_state = if auth.status == AuthStatus::PasskeyEnrollmentRequired {
                "passkey_required"
            } else {
                "active"
            };
            format!(
                r##"<button id="muconsole-user-btn" type="button" data-control-id="hdr-user" data-action="hdr-user">@{handle}</button><button id="muconsole-roles-btn" type="button" data-control-id="hdr-role" data-action="hdr-role" aria-label="{roles_plain}"{role_gated_attr}>{roles_html}</button><a id="muconsole-auth-link" href="{href}" data-auth-state="{auth_state}">{}</a>"##,
                escape_html(copy::SIGN_OUT_LABEL),
            )
        }
    };

    let skip_label = escape_html(copy::SKIP_TO_MAIN);
    let aria_label = escape_html(copy::FILTER_ARIA_LABEL);

    format!(
        r##"<div id="muconsole-shell" data-skin="{skin_str}" data-crt-font="{crt_font_slug}" data-font="{font_css}" style="--font-crt:{font_css};--font-active:{font_css}"><style>{styles}</style><header id="muconsole-header"><a class="skip-link" href="#muconsole-dashboard">{skip_label}</a><input id="muconsole-filter" type="search" value="{filter_val}" placeholder="{placeholder}" aria-label="{aria_label}" style="font-size:16px"{filter_disabled_attr} /><div id="muconsole-aria-polite" role="status" aria-live="polite">{polite}</div><div id="muconsole-aria-assertive" role="alert" aria-live="assertive">{assertive}</div>{header_tail}</header><main id="muconsole-dashboard">{content}</main></div>"##
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use mu_console_protocol::{encode, AuthState, CiSubView, DataEvent, SubView, ViewModel};

    #[test]
    fn html_shell_sets_filter_disabled_state_and_passkey_auth_attribute() {
        let mut st = ConsoleState::new();
        let unresolved_html = render_html_shell("<svg></svg>", &st);
        let unresolved_doc = roxmltree::Document::parse(&unresolved_html).expect("valid XML");
        let filter_unresolved = unresolved_doc
            .descendants()
            .find(|n| n.attribute("id") == Some("muconsole-filter"))
            .expect("#muconsole-filter present");
        assert_eq!(filter_unresolved.attribute("disabled"), Some("disabled"));
        assert_eq!(filter_unresolved.attribute("aria-disabled"), Some("true"));
        assert_eq!(
            filter_unresolved.attribute("placeholder"),
            Some("Resolving session...")
        );

        let passkey_vm = ViewModel {
            auth: AuthState {
                status: AuthStatus::PasskeyEnrollmentRequired,
                github_login: Some("newop".into()),
                passkey_count: 0,
                passkey_enrollment_required: true,
                ..AuthState::default()
            },
            ..ViewModel::default()
        };
        st.on_worker_frame(&encode(&DataEvent::Snapshot(passkey_vm)).expect("encode"));
        st.apply_hash("#/ci/logs");
        let passkey_html = render_html_shell("<svg></svg>", &st);
        let passkey_doc = roxmltree::Document::parse(&passkey_html).expect("valid XML");
        let auth_link = passkey_doc
            .descendants()
            .find(|n| n.attribute("id") == Some("muconsole-auth-link"))
            .expect("#muconsole-auth-link present");
        assert_eq!(
            auth_link.attribute("data-auth-state"),
            Some("passkey_required")
        );
        let filter_passkey = passkey_doc
            .descendants()
            .find(|n| n.attribute("id") == Some("muconsole-filter"))
            .expect("#muconsole-filter present");
        assert_eq!(filter_passkey.attribute("disabled"), None);
        assert_eq!(
            filter_passkey.attribute("placeholder"),
            Some("Filter public repos, branches, or jobs...")
        );

        st.apply_hash("#/platform/users");
        let blocked_html = render_html_shell("<svg></svg>", &st);
        let blocked_doc = roxmltree::Document::parse(&blocked_html).expect("valid XML");
        let filter_blocked = blocked_doc
            .descendants()
            .find(|n| n.attribute("id") == Some("muconsole-filter"))
            .expect("#muconsole-filter present");
        assert_eq!(filter_blocked.attribute("disabled"), Some("disabled"));
        assert_eq!(filter_blocked.attribute("aria-disabled"), Some("true"));
        assert_eq!(
            filter_blocked.attribute("placeholder"),
            Some("Enroll passkey to unlock...")
        );

        let active_vm = ViewModel {
            auth: AuthState {
                status: AuthStatus::Active,
                github_login: Some("ops".into()),
                passkey_count: 1,
                active_policies: vec!["system:dashboard.read".into(), "ci:queue.read".into()],
                ..AuthState::default()
            },
            ..ViewModel::default()
        };
        st.on_worker_frame(&encode(&DataEvent::Snapshot(active_vm)).expect("encode"));
        st.apply_hash("#/ci/scorecards");
        let active_html = render_html_shell("<svg></svg>", &st);
        let active_doc = roxmltree::Document::parse(&active_html).expect("valid XML");
        let active_link = active_doc
            .descendants()
            .find(|n| n.attribute("id") == Some("muconsole-auth-link"))
            .expect("#muconsole-auth-link present");
        assert_eq!(active_link.attribute("data-auth-state"), Some("active"));
        assert_eq!(
            filter_placeholder_for_state(&st),
            "Filter repos, branches, or status..."
        );
        let _ = SubView::Ci(CiSubView::Scorecards);
    }
}
