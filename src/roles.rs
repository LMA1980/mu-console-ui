// SPDX-License-Identifier: AGPL-3.0-only
//! Header `+role[seconds_remaining][idle_timer]` formatting and `Timed` lease lifecycle helpers.

use crate::copy;
use crate::router::can_access_subview;
use mu_console_protocol::{
    Area, AuthState, DashboardSubView, RoleActivationKind, RoleBadgeView, RoleLeaseView, Route,
    SubView,
};

/// Default character budget for [`format_active_roles`] when `max_chars == 0` is unset externally.
pub const DEFAULT_ACTIVE_ROLES_MAX_CHARS: u32 =
    mu_console_protocol::DEFAULT_MAX_ACTIVE_ROLES_HEADER_CHARS;

fn truncate_chars(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let total = s.chars().count();
    if total <= max_chars {
        return s.to_owned();
    }
    if max_chars == 1 {
        return "…".to_owned();
    }
    let mut out: String = s.chars().take(max_chars - 1).collect();
    out.push('…');
    out
}

fn format_role_segment(badge: &RoleBadgeView) -> String {
    let label = badge.effective_label();
    let display = if label.is_empty() {
        badge.role_id.as_str()
    } else {
        label
    };
    let mut segment = format!("+{display}");
    if matches!(badge.activation.kind, RoleActivationKind::Timed) || badge.lease.is_some() {
        if let Some(lease) = &badge.lease {
            let rem = RoleLeaseView::format_mm_ss(lease.seconds_remaining);
            let idle = RoleLeaseView::format_mm_ss(lease.idle_timer);
            segment.push_str(&format!("[{rem}][{idle}]"));
        }
    }
    segment
}

/// Format active roles for the signed-in `+role[seconds_remaining][idle_timer]` header button.
///
/// Joins active role `display_name` (falling back to `role_id`) with `+`, appends
/// `[MM:SS][MM:SS]` (`[seconds_remaining][idle_timer]`) per active `Timed` lease, and when
/// exceeding `max_chars` truncates strictly on UTF-8 char boundaries (`chars().count()`) with
/// a trailing `…(+N)` overflow indicator for hidden roles.
#[must_use]
pub fn format_active_roles(roles: &[RoleBadgeView], max_chars: u32) -> String {
    let cap = max_chars as usize;
    if cap == 0 {
        return String::new();
    }
    if roles.is_empty() {
        return truncate_chars(copy::ROLE_HEADER_EMPTY, cap);
    }

    let segments: Vec<String> = roles.iter().map(format_role_segment).collect();
    let full = segments.concat();
    if full.chars().count() <= cap {
        return full;
    }

    for keep in (1..segments.len()).rev() {
        let hidden = segments.len() - keep;
        let suffix = format!("…(+{hidden})");
        let suffix_len = suffix.chars().count();
        let prefix_len: usize = segments[..keep].iter().map(|s| s.chars().count()).sum();
        if prefix_len + suffix_len <= cap {
            let mut out = segments[..keep].concat();
            out.push_str(&suffix);
            return out;
        }
    }

    if segments.len() > 1 {
        let suffix = format!("…(+{})", segments.len() - 1);
        let suffix_len = suffix.chars().count();
        if suffix_len < cap {
            let mut out: String = segments[0].chars().take(cap - suffix_len).collect();
            out.push_str(&suffix);
            return out;
        }
        return suffix.chars().take(cap).collect();
    }

    truncate_chars(&segments[0], cap)
}

fn escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    mu_console_theme::svg::escape_into(&mut out, s);
    out
}

struct RoleSegment {
    role_id: String,
    plain_prefix: String,
    plain_timer: Option<String>,
    plain_full: String,
    html_full: String,
}

fn build_role_segment(badge: &RoleBadgeView) -> RoleSegment {
    let label = badge.effective_label();
    let display = if label.is_empty() {
        badge.role_id.as_str()
    } else {
        label
    };
    let plain_prefix = format!("+{display}");
    let plain_timer =
        if matches!(badge.activation.kind, RoleActivationKind::Timed) || badge.lease.is_some() {
            badge.lease.as_ref().map(|lease| {
                format!(
                    "[{}][{}]",
                    RoleLeaseView::format_mm_ss(lease.seconds_remaining),
                    RoleLeaseView::format_mm_ss(lease.idle_timer)
                )
            })
        } else {
            None
        };
    let plain_full = match &plain_timer {
        Some(t) => format!("{plain_prefix}{t}"),
        None => plain_prefix.clone(),
    };
    let html_full = match &plain_timer {
        Some(t) => format!(
            "{}<span class=\"mu-lease-timer\" data-lease-timer=\"hdr:{}\">{}</span>",
            escape_text(&plain_prefix),
            escape_text(&badge.role_id),
            escape_text(t),
        ),
        None => escape_text(&plain_prefix),
    };
    RoleSegment {
        role_id: badge.role_id.clone(),
        plain_prefix,
        plain_timer,
        plain_full,
        html_full,
    }
}

fn format_truncated_segment_html(seg: &RoleSegment, content_budget: usize, tail: &str) -> String {
    let prefix_chars = seg.plain_prefix.chars().count();
    if content_budget <= prefix_chars {
        let head: String = seg.plain_prefix.chars().take(content_budget).collect();
        return format!("{}{}", escape_text(&head), escape_text(tail));
    }
    if let Some(timer) = &seg.plain_timer {
        let avail = content_budget - prefix_chars;
        let timer_slice: String = timer.chars().take(avail).collect();
        return format!(
            "{}<span class=\"mu-lease-timer\" data-lease-timer=\"hdr-trunc:{}:{avail}\">{}</span>{}",
            escape_text(&seg.plain_prefix),
            escape_text(&seg.role_id),
            escape_text(&timer_slice),
            escape_text(tail),
        );
    }
    let head: String = seg.plain_full.chars().take(content_budget).collect();
    format!("{}{}", escape_text(&head), escape_text(tail))
}

/// Format active roles as HTML wrapping only `[seconds_remaining][idle_timer]` values in
/// `<span class="mu-lease-timer" data-lease-timer="...">` so timer ticks never redraw static badges.
#[must_use]
pub fn format_active_roles_html(roles: &[RoleBadgeView], max_chars: u32) -> String {
    let cap = max_chars as usize;
    if cap == 0 {
        return String::new();
    }
    if roles.is_empty() {
        return escape_text(&truncate_chars(copy::ROLE_HEADER_EMPTY, cap));
    }

    let segs: Vec<RoleSegment> = roles.iter().map(build_role_segment).collect();
    let full_chars: usize = segs.iter().map(|s| s.plain_full.chars().count()).sum();
    if full_chars <= cap {
        return segs.iter().map(|s| s.html_full.as_str()).collect();
    }

    for keep in (1..segs.len()).rev() {
        let hidden = segs.len() - keep;
        let suffix = format!("…(+{hidden})");
        let suffix_len = suffix.chars().count();
        let prefix_len: usize = segs[..keep]
            .iter()
            .map(|s| s.plain_full.chars().count())
            .sum();
        if prefix_len + suffix_len <= cap {
            let mut out: String = segs[..keep].iter().map(|s| s.html_full.as_str()).collect();
            out.push_str(&escape_text(&suffix));
            return out;
        }
    }

    if segs.len() > 1 {
        let suffix = format!("…(+{})", segs.len() - 1);
        let suffix_len = suffix.chars().count();
        if suffix_len < cap {
            return format_truncated_segment_html(&segs[0], cap - suffix_len, &suffix);
        }
        return escape_text(&suffix.chars().take(cap).collect::<String>());
    }

    if cap == 1 {
        return "…".to_owned();
    }
    format_truncated_segment_html(&segs[0], cap - 1, "…")
}

/// Resolve the live display text for a `[data-lease-timer="..."]` element key from `auth`.
#[must_use]
pub fn resolve_lease_timer_text(auth: &AuthState, key: &str) -> Option<String> {
    if let Some(role_id) = key.strip_prefix("hdr:") {
        let lease = auth.leases.iter().find(|l| l.role_id == role_id)?;
        return Some(format!(
            "[{}][{}]",
            RoleLeaseView::format_mm_ss(lease.seconds_remaining),
            RoleLeaseView::format_mm_ss(lease.idle_timer),
        ));
    }
    if let Some(rest) = key.strip_prefix("hdr-trunc:") {
        let (role_id, avail_str) = rest.rsplit_once(':')?;
        let avail: usize = avail_str.parse().ok()?;
        let lease = auth.leases.iter().find(|l| l.role_id == role_id)?;
        let full = format!(
            "[{}][{}]",
            RoleLeaseView::format_mm_ss(lease.seconds_remaining),
            RoleLeaseView::format_mm_ss(lease.idle_timer),
        );
        return Some(full.chars().take(avail).collect());
    }
    if let Some(role_id) = key
        .strip_prefix("renew-rem:")
        .or_else(|| key.strip_prefix("dlg-rem:"))
    {
        let lease = auth.leases.iter().find(|l| l.role_id == role_id)?;
        return Some(RoleLeaseView::format_mm_ss(lease.seconds_remaining));
    }
    if let Some(rest) = key.strip_prefix("dlg-rem-trunc:") {
        let (role_id, avail_str) = rest.rsplit_once(':')?;
        let avail: usize = avail_str.parse().ok()?;
        let lease = auth.leases.iter().find(|l| l.role_id == role_id)?;
        let full = RoleLeaseView::format_mm_ss(lease.seconds_remaining);
        return Some(full.chars().take(avail).collect());
    }
    if let Some(role_id) = key.strip_prefix("renew-idle:") {
        let lease = auth.leases.iter().find(|l| l.role_id == role_id)?;
        return Some(RoleLeaseView::format_mm_ss(lease.idle_timer));
    }
    if let Some(role_id) = key.strip_prefix("profile-secs:") {
        let lease = auth.leases.iter().find(|l| l.role_id == role_id)?;
        return Some(format!("{}s", lease.seconds_remaining));
    }
    None
}

/// Advance local `Timed` lease countdowns to `now_ms`.
///
/// Returns the IDs of `Timed` roles that reached `00:00` and were deactivated from `auth`.
/// Non-expired `Timed` roles, `Elective` roles, and `PinnedBaseline` remain intact.
pub fn expire_timed_leases(auth: &mut AuthState, now_ms: u64) -> Vec<String> {
    if auth.leases.is_empty() {
        return Vec::new();
    }
    let mut expired_ids = Vec::new();
    let mut active_leases = Vec::with_capacity(auth.leases.len());
    for mut lease in auth.leases.drain(..) {
        if lease.tick_at(now_ms) && !lease.is_expired() {
            active_leases.push(lease);
        } else {
            expired_ids.push(lease.role_id.clone());
        }
    }
    auth.leases = active_leases;

    for badge in &mut auth.active_roles {
        if let Some(live) = auth.leases.iter().find(|l| l.role_id == badge.role_id) {
            badge.lease = Some(live.clone());
        }
    }

    if !expired_ids.is_empty() {
        auth.active_roles
            .retain(|b| !expired_ids.iter().any(|id| id == &b.role_id));
    }
    expired_ids
}

/// Apply explicit user activity locally onto active `Timed` leases (SSE ticks must never call this).
pub fn touch_active_leases(auth: &mut AuthState, now_ms: u64) -> bool {
    let mut changed = false;
    for lease in &mut auth.leases {
        if !lease.is_expired() && lease.idle_ttl_secs > 0 {
            lease.last_activity_at_ms = now_ms;
            let raw_idle = now_ms.saturating_add(lease.idle_ttl_secs.saturating_mul(1_000));
            lease.idle_expires_at_ms = if lease.hard_expires_at_ms > 0 {
                raw_idle.min(lease.hard_expires_at_ms)
            } else {
                raw_idle
            };
            if lease.hard_expires_at_ms > 0 {
                let _ = lease.tick_at(now_ms);
            } else {
                lease.idle_timer = lease.idle_ttl_secs.min(lease.seconds_remaining);
            }
            changed = true;
        }
    }
    if changed {
        for badge in &mut auth.active_roles {
            if let Some(live) = auth.leases.iter().find(|l| l.role_id == badge.role_id) {
                badge.lease = Some(live.clone());
            }
        }
    }
    changed
}

/// Redirect `route` to `#/dashboard/overview` strictly when remaining active roles lack
/// minimal access to `route.subview`. Returns `true` if `route` was changed.
pub fn redirect_if_lacking_access(auth: &AuthState, route: &mut Route) -> bool {
    if can_access_subview(auth, route.subview) {
        return false;
    }
    let target = Route {
        area: Area::Dashboard,
        subview: SubView::Dashboard(DashboardSubView::Overview),
        repo: route.repo.clone(),
        submodule: None,
        branch: None,
        job: None,
        trace_id: None,
        filter: None,
    };
    *route = target;
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use mu_console_protocol::{AuthStatus, RoleActivationDto};

    fn badge(id: &str, name: Option<&str>, timed: Option<(u64, u64)>) -> RoleBadgeView {
        match timed {
            Some((rem, idle)) => RoleBadgeView {
                role_id: id.into(),
                display_name: name.map(str::to_owned),
                activation: RoleActivationDto::timed(rem, idle),
                lease: Some(RoleLeaseView {
                    role_id: id.into(),
                    display_name: name.map(str::to_owned),
                    seconds_remaining: rem,
                    idle_timer: idle,
                    hard_ttl_secs: rem,
                    idle_ttl_secs: idle,
                    activated_at_ms: 0,
                    last_activity_at_ms: 0,
                    hard_expires_at_ms: rem * 1_000,
                    idle_expires_at_ms: idle * 1_000,
                    ..RoleLeaseView::default()
                }),
            },
            None => RoleBadgeView {
                role_id: id.into(),
                display_name: name.map(str::to_owned),
                activation: RoleActivationDto::elective(),
                lease: None,
            },
        }
    }

    #[test]
    fn format_active_roles_handles_empty_single_exact_fit_timed_overflow_and_utf8() {
        // 1. Empty
        assert_eq!(format_active_roles(&[], 120), "+role");
        assert_eq!(format_active_roles(&[], 0), "");

        // 2. Single (fallback to role_id when display_name unset)
        let viewer = badge("viewer", None, None);
        assert_eq!(
            format_active_roles(std::slice::from_ref(&viewer), 120),
            "+viewer"
        );

        // 3. Timed countdown badges `[MM:SS][MM:SS]`
        let op = badge("ci-op", Some("CI Operator"), Some((252, 119)));
        assert_eq!(
            format_active_roles(&[viewer.clone(), op.clone()], 120),
            "+viewer+CI Operator[04:12][01:59]"
        );

        // 4. Multi-role exact fit vs 1-char tighter custom cap
        let exact = "+viewer+CI Operator[04:12][01:59]";
        let exact_len = u32::try_from(exact.chars().count()).unwrap();
        assert_eq!(
            format_active_roles(&[viewer.clone(), op.clone()], exact_len),
            exact
        );
        assert_eq!(
            format_active_roles(&[viewer.clone(), op.clone()], exact_len - 1),
            "+viewer…(+1)"
        );

        // 5. Overflow multiple hidden roles
        let admin = badge("admin", Some("Platform Admin"), Some((60, 30)));
        assert_eq!(
            format_active_roles(&[viewer, op, admin], 15),
            "+viewer…(+2)"
        );

        // 6. Multi-byte UTF-8 boundary safety (emoji & accented labels)
        let utf8_a = badge("r1", Some("🦀Observability🔥Ops"), None);
        let utf8_b = badge("r2", Some("Déploiement"), None);
        for cap in 1..=25 {
            let formatted = format_active_roles(&[utf8_a.clone(), utf8_b.clone()], cap);
            assert!(formatted.chars().count() <= cap as usize);
        }
    }

    #[test]
    fn timed_expiry_drops_only_expired_role_and_conditionally_redirects() {
        let mut auth = AuthState {
            status: AuthStatus::Active,
            passkey_count: 1,
            active_roles: vec![
                badge("viewer", Some("Viewer"), None),
                RoleBadgeView {
                    role_id: "ab-op".into(),
                    display_name: Some("AB Operator".into()),
                    activation: RoleActivationDto::timed(10, 5),
                    lease: Some(RoleLeaseView {
                        role_id: "ab-op".into(),
                        hard_expires_at_ms: 10_000,
                        idle_expires_at_ms: 5_000,
                        seconds_remaining: 10,
                        idle_timer: 5,
                        hard_ttl_secs: 10,
                        idle_ttl_secs: 5,
                        ..RoleLeaseView::default()
                    }),
                },
            ],
            leases: vec![RoleLeaseView {
                role_id: "ab-op".into(),
                hard_expires_at_ms: 10_000,
                idle_expires_at_ms: 5_000,
                seconds_remaining: 10,
                idle_timer: 5,
                hard_ttl_secs: 10,
                idle_ttl_secs: 5,
                ..RoleLeaseView::default()
            }],
            active_policies: vec![
                "system:dashboard.read".into(),
                "ci:queue.read".into(),
                "ab:slots.read".into(),
            ],
            ..AuthState::default()
        };

        // At idle expiry (5_000ms), `ab-op` expires; viewer remains.
        let expired = expire_timed_leases(&mut auth, 5_000);
        assert_eq!(expired, vec!["ab-op"]);
        assert_eq!(auth.active_roles.len(), 1);
        assert_eq!(auth.active_roles[0].role_id, "viewer");

        // Simulate baseline-only policy remaining after expiry:
        auth.active_policies = vec!["system:dashboard.read".into(), "ci:queue.read".into()];

        // If operator was viewing CI QueueLogs (permitted by baseline): stays on CI.
        let mut ci_route =
            Route::for_subview(SubView::Ci(mu_console_protocol::CiSubView::QueueLogs));
        assert!(!redirect_if_lacking_access(&auth, &mut ci_route));
        assert_eq!(ci_route.area, Area::Ci);

        // If operator was viewing AbDeploy (requires expired role): redirects to Dashboard.
        let mut ab_route = Route::for_area(Area::AbDeploy);
        assert!(redirect_if_lacking_access(&auth, &mut ab_route));
        assert_eq!(ab_route.area, Area::Dashboard);
    }

    fn strip_html_tags(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut in_tag = false;
        for ch in s.chars() {
            match ch {
                '<' => in_tag = true,
                '>' => in_tag = false,
                _ if !in_tag => out.push(ch),
                _ => {}
            }
        }
        out
    }

    #[test]
    fn format_active_roles_html_matches_plain_budget_and_resolves_all_timer_keys() {
        let viewer = badge("viewer", Some("Viewer"), None);
        let super_op = badge("super-operator", Some("Super Operator"), Some((899, 118)));
        let roles = [super_op.clone(), viewer.clone()];

        for cap in 0..=60 {
            let html = format_active_roles_html(&roles, cap);
            let plain = format_active_roles(&roles, cap);
            assert_eq!(strip_html_tags(&html), escape_text(&plain), "cap={cap}");

            let single_html = format_active_roles_html(std::slice::from_ref(&super_op), cap);
            let single_plain = format_active_roles(std::slice::from_ref(&super_op), cap);
            assert_eq!(
                strip_html_tags(&single_html),
                escape_text(&single_plain),
                "single cap={cap}"
            );
        }

        let auth = AuthState {
            status: AuthStatus::Active,
            leases: vec![super_op.lease.unwrap()],
            ..AuthState::default()
        };
        assert_eq!(
            resolve_lease_timer_text(&auth, "hdr:super-operator").as_deref(),
            Some("[14:59][01:58]")
        );
        assert_eq!(
            resolve_lease_timer_text(&auth, "hdr-trunc:super-operator:7").as_deref(),
            Some("[14:59]")
        );
        assert_eq!(
            resolve_lease_timer_text(&auth, "renew-rem:super-operator").as_deref(),
            Some("14:59")
        );
        assert_eq!(
            resolve_lease_timer_text(&auth, "renew-idle:super-operator").as_deref(),
            Some("01:58")
        );
        assert_eq!(
            resolve_lease_timer_text(&auth, "dlg-rem:super-operator").as_deref(),
            Some("14:59")
        );
        assert_eq!(
            resolve_lease_timer_text(&auth, "profile-secs:super-operator").as_deref(),
            Some("899s")
        );
        assert_eq!(resolve_lease_timer_text(&auth, "hdr:missing"), None);
    }
}
