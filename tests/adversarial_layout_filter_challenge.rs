// SPDX-License-Identifier: AGPL-3.0-only
//! Adversarial Challenge Test Harness for Milestone 6.7:
//! Layout Invariants, Font-Size 16px Invariant, and Filter Robustness.
//!
//! Stress-tests:
//! 1. Layout invariants: Zero scrollbars across viewports (320x568 up to 5120x1440).
//! 2. Font-size 16px invariant on `#muconsole-filter` across all font/dyslexic states.
//! 3. Extreme inputs for `filter_query`: Unicode, XSS payloads (`<script>`, `"><img src=x>`),
//!    long strings (10,000 chars), quotes, newlines. Evaluates HTML escaping and markup integrity.
//! 4. HTML shell element escaping integrity for aria status and auth attributes.
#![cfg(not(miri))]

use mu_console_layout::{solve, Area, SafeArea, Viewport};
use mu_console_protocol::CrtFont;
use mu_console_theme::css::tokens_css;
use mu_console_theme::metrics::{HURMIT, OPEN_DYSLEXIC};
use mu_console_ui::components::render_html_shell;
use mu_console_ui::state::ConsoleState;

#[test]
fn test_layout_zero_scrollbars_and_css_invariants() {
    let css = tokens_css();

    // 1. Verify CSS rules enforce zero scroll leakage on html, body, and #muconsole-shell
    assert!(
        css.contains("html,body{margin:0;padding:0;height:100vh;overflow:hidden;height:100dvh;"),
        "tokens_css() must enforce overflow:hidden and 100dvh on html,body"
    );
    assert!(
        css.contains("#muconsole-shell{display:flex;flex-direction:column;height:100vh;height:100dvh;overflow:hidden;margin:0;padding:0}"),
        "tokens_css() must enforce flex column, 100dvh, and overflow:hidden on #muconsole-shell"
    );
    assert!(
        css.contains(
            "#muconsole-dashboard{flex:1 1 auto;overflow:hidden;position:relative;display:flex}"
        ),
        "tokens_css() must enforce flex:1 1 auto and overflow:hidden on #muconsole-dashboard"
    );
    assert!(
        css.contains("#muconsole-dashboard>svg{width:100%;height:100%;display:block}"),
        "tokens_css() must enforce 100% width and height on #muconsole-dashboard>svg"
    );

    // 2. Test layout solver across viewports: small mobile, tablet, desktop, ultrawide
    let target_viewports = [
        (320.0, 568.0, "iPhone SE 1st gen"),
        (375.0, 667.0, "iPhone SE 2nd/3rd gen"),
        (390.0, 844.0, "iPhone 14 / modern phone"),
        (430.0, 932.0, "iPhone 14 Pro Max"),
        (768.0, 1024.0, "iPad Mini portrait"),
        (834.0, 1194.0, "iPad Pro 11 portrait"),
        (1024.0, 768.0, "iPad Mini landscape"),
        (1280.0, 800.0, "Laptop / Compact"),
        (1920.0, 1080.0, "Desktop FHD"),
        (2560.0, 1440.0, "Desktop QHD"),
        (3440.0, 1440.0, "Ultrawide 21:9"),
        (5120.0, 1440.0, "Super Ultrawide 32:9"),
    ];

    let fonts = [&HURMIT, &OPEN_DYSLEXIC];
    let areas = [
        Area::Ci,
        Area::Cd,
        Area::Registry,
        Area::Observe,
        Area::Fleet,
    ];

    for (w, h, label) in target_viewports {
        let vp = Viewport::new(w, h);
        let safe = SafeArea::default();
        for &font in &fonts {
            for area in areas {
                let frames = solve(vp, safe, area, font);
                assert!(
                    frames.canvas.x >= 0.0 && frames.canvas.right() <= w + 0.001,
                    "{label} ({w}x{h}): canvas width exceeds viewport: {} > {w}",
                    frames.canvas.right()
                );
                assert!(
                    frames.canvas.y >= 0.0 && frames.canvas.bottom() <= h + 0.001,
                    "{label} ({w}x{h}): canvas height exceeds viewport: {} > {h}",
                    frames.canvas.bottom()
                );
                for p in &frames.panels {
                    assert!(
                        p.plate.right() <= frames.canvas.right() + 0.001,
                        "{label}: panel plate right {} exceeds canvas right {}",
                        p.plate.right(),
                        frames.canvas.right()
                    );
                    assert!(
                        p.plate.bottom() <= frames.canvas.bottom() + 0.001,
                        "{label}: panel plate bottom {} exceeds canvas bottom {}",
                        p.plate.bottom(),
                        frames.canvas.bottom()
                    );
                }
            }
        }
    }
}

#[test]
fn test_font_size_16px_invariant_across_all_theme_modes() {
    let css = tokens_css();

    // Check tokens_css() definition
    assert!(
        css.contains("#muconsole-filter{font-family:var(--font-crt);font-size:16px;"),
        "tokens_css() must enforce font-size:16px on #muconsole-filter"
    );

    // Verify across all theme/font permutations
    let permutations = [
        (CrtFont::Hurmit, false),
        (CrtFont::Hurmit, true),
        (CrtFont::OpenDyslexic, false),
        (CrtFont::OpenDyslexic, true),
    ];

    for (font, lazy_dyslexic) in permutations {
        let mut st = ConsoleState::new();
        st.set_font(font);
        st.lazy_dyslexic_loaded = lazy_dyslexic;

        let html = render_html_shell("<svg id=\"canvas\"></svg>", &st);
        let doc = roxmltree::Document::parse(&html).expect("must parse XML");
        let filter_node = doc
            .descendants()
            .find(|n| n.attribute("id") == Some("muconsole-filter"))
            .expect("#muconsole-filter must exist");

        // Verify style attribute on element
        let style = filter_node.attribute("style").unwrap_or("");
        assert!(
            style.contains("font-size:16px") || style.contains("font-size: 16px"),
            "Font {font:?} lazy={lazy_dyslexic}: inline style must specify font-size:16px, got: {style}"
        );

        // Verify embedded <style> tag
        let style_node = doc
            .descendants()
            .find(|n| n.has_tag_name("style"))
            .expect("<style> tag must exist in shell");
        let style_text = style_node.text().unwrap_or("");
        assert!(
            style_text.contains("#muconsole-filter{font-size:16px;}"),
            "Font {font:?} lazy={lazy_dyslexic}: embedded style must declare #muconsole-filter{{font-size:16px;}}"
        );

        if lazy_dyslexic {
            assert!(
                style_text.contains("@font-face{font-family:'OpenDyslexicM Nerd Font Mono'"),
                "When lazy_dyslexic_loaded is true, @font-face must be present in embedded <style>"
            );
        }
    }
}

/// Verify that render_html_shell properly escapes filter_query, preventing XSS breakout
/// and preserving well-formed HTML shell markup.
#[test]
fn test_filter_query_adversarial_vulnerabilities_demonstration() {
    let test_cases = [
        (
            "xss_breakout_img_onerror",
            "\"><img src=x onerror=alert(1)>",
            "&quot;&gt;&lt;img src=x onerror=alert(1)&gt;",
        ),
        ("attribute_quote_escape", "search\"term", "search&quot;term"),
        (
            "raw_ampersand_and_tag",
            "repo & <script>alert('xss')</script>",
            "repo &amp; &lt;script&gt;alert(&#39;xss&#39;)&lt;/script&gt;",
        ),
    ];

    for (name, payload, expected_escaped) in test_cases {
        let mut st = ConsoleState::new();
        st.filter_query = payload.to_owned();

        let raw_html = render_html_shell("<svg></svg>", &st);

        let input_snippet = raw_html
            .lines()
            .find(|l| l.contains("muconsole-filter"))
            .unwrap_or(&raw_html);

        // Confirm escaped entity is present in HTML snippet
        assert!(
            input_snippet.contains(expected_escaped),
            "Expected escaped payload '{expected_escaped}' in snippet: {input_snippet}"
        );
        // Confirm raw breakout is NOT present
        assert!(
            !input_snippet.contains(payload),
            "Raw unescaped payload '{payload}' must NOT appear in HTML snippet: {input_snippet}"
        );

        // Confirm roxmltree parses cleanly and decodes back to original input
        let doc = roxmltree::Document::parse(&raw_html)
            .unwrap_or_else(|e| panic!("Failed to parse XML for {name}: {e}"));
        let filter_node = doc
            .descendants()
            .find(|n| n.attribute("id") == Some("muconsole-filter"))
            .expect("#muconsole-filter must exist");
        assert_eq!(filter_node.attribute("value"), Some(payload));
    }
}

/// Reference verification: Demonstrates that applying mu_console_theme::svg::escape_into
/// to filter_query renders 100% valid XML/HTML shell markup across all extreme inputs.
#[test]
fn test_sanitized_reference_oracle_passes_all_extreme_inputs() {
    let cases = [
        "model-runner",
        "🦀 🚀 日本語 漢字 العربية עברית \u{202E}RTL",
        "repo\"name",
        "repo'name",
        "<script>alert('xss')</script>",
        "\"><img src=x onerror=alert(1)>",
        "\" autofocus onfocus=\"alert(1)",
        "multi\nline\r\nfilter",
        &"A".repeat(10_000),
        "</header><script>alert(1)</script>",
        "foo & bar &amp; &lt; &gt;",
    ];

    for payload in cases {
        // Build shell using raw query directly — render_html_shell performs escaping internally
        let mut st = ConsoleState::new();
        st.filter_query = payload.to_string();

        let html = render_html_shell("<svg id=\"canvas\"></svg>", &st);

        // Verify it parses cleanly with zero XML errors
        let doc = roxmltree::Document::parse(&html)
            .unwrap_or_else(|e| panic!("Oracle failed on payload {:?}: {e}", payload));

        let filter_node = doc
            .descendants()
            .find(|n| n.attribute("id") == Some("muconsole-filter"))
            .expect("#muconsole-filter must exist");

        // The parsed value attribute matches the original payload (un-escaped by XML parser)
        let parsed_val = filter_node.attribute("value").unwrap_or("");
        // Note: roxmltree normalizes attribute whitespace (\n -> space), but non-whitespace matches
        if !payload.contains('\n') && !payload.contains('\r') {
            assert_eq!(
                parsed_val, payload,
                "Parsed attribute value must match original input"
            );
        }

        // Header children remain strictly intact
        let header = doc
            .descendants()
            .find(|n| n.attribute("id") == Some("muconsole-header"))
            .expect("header must exist");

        let sibling_tags: Vec<_> = header
            .children()
            .filter(|n| n.is_element())
            .map(|n| n.tag_name().name())
            .collect();

        assert_eq!(
            sibling_tags,
            vec!["a", "input", "div", "div", "a"],
            "Header DOM structure must remain unaltered regardless of input payload"
        );
    }
}

/// Comprehensive adversarial fuzzing suite:
/// Tests quote breakouts, unbalanced tags, control characters, entity expansion,
/// and dynamic injection into aria-live and auth fields.
#[test]
fn test_adversarial_layout_filter_fuzzing() {
    let adversarial_fuzz_cases = [
        // 1. Quote combinations & breakouts
        "\"\"\"\"\"\"\"\"\"\"",
        "''''''''''",
        "\"'\"'\"'\"'\"'",
        "\\\"\\\"\\\"",
        "\" onfocus=\"alert(1)\" \"",
        "\" autofocus style=\"display:none",
        "&#34;&#39;&quot;&apos;",
        "\"`'${alert(1)}'`\"",
        // 2. Unbalanced and nested HTML/XML tags
        "<<<<<<<<<<<<<<<<<<<<",
        ">>>>>>>>>>>>>>>>>>>>",
        "<<>>><<<>>><<<>>>",
        "<input type=\"text\" value=\"broken\">",
        "</input></header></div><script>evil()</script>",
        "<!-- <comment> -->",
        "<!-- unclosed comment",
        "--> unopen comment",
        "<![CDATA[ <not xml> ]]>",
        "<!DOCTYPE html>",
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>",
        "<svg onload=\"alert(document.domain)\">",
        "<img src=\"x\" onerror=\"alert(1)\"/>",
        // 3. XML Entity attacks & ampersand edge cases
        "&",
        "&&&",
        "&amp;&amp;&amp;",
        "&notanentity;",
        "&#x1F4A9;",
        "&#999999999;",
        "&amp;lt;script&amp;gt;",
        // 4. Whitespace, control chars & unicode boundaries
        "\t\r\n   ",
        "\u{200B}\u{200C}\u{200D}\u{FEFF}", // zero-width & BOM
        "\u{202E}dlrow olleh\u{202C}",      // bidi override
        "﷽",                                // Basmala ligature (single char, very wide)
        "👩‍👩‍👧‍👦",                               // family emoji (multi-codepoint ZWJ sequence)
        "Z\u{0300}\u{0301}\u{0302}\u{0303}\u{0304}\u{0305}\u{0306}", // Zalgo combining marks
        // 5. Huge payloads
        &"X".repeat(50_000),
        &"\"<>&'".repeat(2_000),
    ];

    for (idx, payload) in adversarial_fuzz_cases.iter().enumerate() {
        let mut st = ConsoleState::new();
        st.filter_query = payload.to_string();

        let html = render_html_shell("<svg id=\"canvas\"></svg>", &st);

        // Verify XML document parsing
        let doc = roxmltree::Document::parse(&html).unwrap_or_else(|e| {
            panic!("Fuzz case #{idx} failed to parse XML on payload {payload:?}: {e}")
        });

        // Verify filter node exists and value is intact
        let filter_node = doc
            .descendants()
            .find(|n| n.attribute("id") == Some("muconsole-filter"))
            .expect("#muconsole-filter must exist");
        assert!(filter_node.attribute("value").is_some());

        // Structure invariant: Header children must remain intact
        let header = doc
            .descendants()
            .find(|n| n.attribute("id") == Some("muconsole-header"))
            .expect("header must exist");

        let sibling_tags: Vec<_> = header
            .children()
            .filter(|n| n.is_element())
            .map(|n| n.tag_name().name())
            .collect();

        assert_eq!(
            sibling_tags,
            vec!["a", "input", "div", "div", "a"],
            "Fuzz case #{idx}: Header DOM structure was altered by payload {payload:?}"
        );
    }
}

/// Adversarial testing for dynamic aria-live regions and auth states.
#[test]
fn test_adversarial_aria_and_auth_injection() {
    use mu_console_data::engine::Engine;
    use mu_console_protocol::{AuthState, DataEvent, Delta};

    let evil_payloads = [
        "\"><script>alert('aria-xss')</script>",
        "</div><header>",
        "&lt;evil&gt; & \"quotes\"",
        "Special chars: <>&\"' and \u{202E}RTL",
    ];

    for evil in evil_payloads {
        // 1. Test aria-polite and aria-assertive injection
        let mut st = ConsoleState::new();
        st.aria_polite = evil.to_owned();
        st.aria_assertive = evil.to_owned();

        let html = render_html_shell("<svg></svg>", &st);
        let doc = roxmltree::Document::parse(&html)
            .unwrap_or_else(|e| panic!("Aria injection XML parse failed on {evil:?}: {e}"));

        let polite_node = doc
            .descendants()
            .find(|n| n.attribute("id") == Some("muconsole-aria-polite"))
            .expect("aria-polite node must exist");
        assert_eq!(polite_node.text(), Some(evil));

        let assertive_node = doc
            .descendants()
            .find(|n| n.attribute("id") == Some("muconsole-aria-assertive"))
            .expect("aria-assertive node must exist");
        assert_eq!(assertive_node.text(), Some(evil));

        // 2. Test auth injection via github_login in InviteRequired
        let mut st_invite = ConsoleState::new();
        let init_snap = Engine::encode_event(&DataEvent::Snapshot(
            mu_console_protocol::ViewModel::default(),
        ))
        .expect("encode snapshot");
        st_invite.on_worker_frame(&init_snap);

        let invite_patch = Engine::encode_event(&DataEvent::Patch(vec![Delta::Auth(
            AuthState::InviteRequired {
                github_login: evil.to_owned(),
            },
        )]))
        .expect("encode Delta::Auth InviteRequired");
        st_invite.on_worker_frame(&invite_patch);

        let html_invite = render_html_shell("<svg></svg>", &st_invite);
        let doc_invite = roxmltree::Document::parse(&html_invite)
            .unwrap_or_else(|e| panic!("Auth invite injection XML parse failed on {evil:?}: {e}"));
        let auth_link = doc_invite
            .descendants()
            .find(|n| n.attribute("id") == Some("muconsole-auth-link"))
            .expect("auth-link node must exist");
        let expected_label = format!("INVITE REQ (@{evil})");
        assert_eq!(auth_link.text(), Some(expected_label.as_str()));

        // 3. Test auth injection via github_login in Operator
        let mut st_operator = ConsoleState::new();
        st_operator.on_worker_frame(&init_snap);

        let op_patch =
            Engine::encode_event(&DataEvent::Patch(vec![Delta::Auth(AuthState::Operator {
                github_login: evil.to_owned(),
                role: None,
            })]))
            .expect("encode Delta::Auth Operator");
        st_operator.on_worker_frame(&op_patch);

        let html_op = render_html_shell("<svg></svg>", &st_operator);
        let doc_op = roxmltree::Document::parse(&html_op).unwrap_or_else(|e| {
            panic!("Auth operator injection XML parse failed on {evil:?}: {e}")
        });
        let auth_link_op = doc_op
            .descendants()
            .find(|n| n.attribute("id") == Some("muconsole-auth-link"))
            .expect("auth-link node must exist");
        let expected_op_label = format!("@{evil} · OPERATOR");
        assert_eq!(auth_link_op.text(), Some(expected_op_label.as_str()));
    }
}

/// Stress-test behavior on ASCII control characters and null bytes.
#[test]
fn test_control_characters_and_xml_spec() {
    // Valid XML whitespace: \t (0x9), \n (0xA), \r (0xD)
    let valid_whitespace = "hello\tworld\nfoo\rbar";
    let mut st = ConsoleState::new();
    st.filter_query = valid_whitespace.to_string();
    let html = render_html_shell("<svg></svg>", &st);
    let doc = roxmltree::Document::parse(&html).expect("valid XML whitespace must parse");
    let filter = doc
        .descendants()
        .find(|n| n.attribute("id") == Some("muconsole-filter"))
        .expect("filter node exists");
    // XML attributes normalize whitespace to space, but must be present
    assert!(filter.attribute("value").is_some());

    // Null byte and ASCII control characters (0x01..0x08, 0x0B..0x0C, 0x0E..0x1F)
    // In XML 1.0, 0x00..0x08 are disallowed. Let's inspect roxmltree behavior:
    let control_chars = [
        ("null_byte", "\0"),
        ("soh_0x01", "\x01"),
        ("bel_0x07", "\x07"),
        ("bs_0x08", "\x08"),
        ("vt_0x0b", "\x0b"),
        ("ff_0x0c", "\x0c"),
        ("esc_0x1b", "\x1b"),
    ];

    for (name, c_str) in control_chars {
        let mut st = ConsoleState::new();
        st.filter_query = format!("prefix{c_str}suffix");
        let html = render_html_shell("<svg></svg>", &st);

        // Check whether roxmltree accepts or rejects per XML 1.0
        let parse_res = roxmltree::Document::parse(&html);
        match parse_res {
            Ok(doc) => {
                // If parser accepts, ensure structure is not corrupted
                let filter = doc
                    .descendants()
                    .find(|n| n.attribute("id") == Some("muconsole-filter"))
                    .expect("filter node must exist");
                assert!(filter.attribute("value").is_some());
            }
            Err(_e) => {
                // roxmltree strictly enforces XML 1.0 disallowed characters
                // (e.g. unknown token / disallowed char), but markup must NOT have tag breakouts
                assert!(
                    !html.contains("<script"),
                    "{name}: Tag breakout must never occur: {html}"
                );
                assert!(
                    html.contains("value=\"prefix"),
                    "{name}: Value attribute must still start with prefix: {html}"
                );
            }
        }
    }
}
