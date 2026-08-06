use std::path::{Path, PathBuf};

fn vendor() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/eql-log-reader")
}

fn read(name: &str) -> String {
    std::fs::read_to_string(vendor().join(name))
        .unwrap_or_else(|err| panic!("vendor/eql-log-reader/{name}: {err}"))
}

fn pin(key: &str) -> String {
    read("upstream.env")
        .lines()
        .find_map(|line| {
            line.strip_prefix(&format!("{key}="))?
                .trim()
                .to_string()
                .into()
        })
        .unwrap_or_else(|| panic!("upstream.env has no {key}"))
}

#[test]
fn the_upstream_pin_is_a_repository_a_tag_and_an_exact_commit() {
    assert!(pin("UPSTREAM_REMOTE").starts_with("https://"));
    assert!(!pin("UPSTREAM_TAG").is_empty());
    let commit = pin("UPSTREAM_COMMIT");
    assert_eq!(
        commit.len(),
        40,
        "a tag can move, a commit cannot: {commit}"
    );
    assert!(commit.chars().all(|c| c.is_ascii_hexdigit()));
}

/// The whole point of the patch is the tool eqld resolves as a sibling; a
/// rename on either side has to fail here rather than at runtime on the rig.
#[test]
fn the_patch_creates_the_tool_eqld_looks_for() {
    let patch = read("headless.patch");
    let created: Vec<&str> = patch
        .lines()
        .filter_map(|line| line.strip_prefix("+++ b/"))
        .collect();
    for stem in [eqld::tools::HEADLESS_STEM, eqld::tools::FIGHTS_STEM] {
        assert!(
            created.contains(&format!("{stem}.py").as_str()),
            "the patch does not add {stem}.py: {created:?}"
        );
    }
    for expected in [
        "eql_quest_cli.py",
        "eql_dps_meter.py",
        "eql_atlas.py",
        "eql_suite.spec",
    ] {
        assert!(
            created.contains(&expected),
            "the patch does not touch {expected}"
        );
    }
}

/// A windowed PyInstaller exe has no valid stdout on Windows, so a headless
/// tool built that way is mute.
#[test]
fn the_patched_spec_builds_the_headless_tools_as_console_binaries() {
    let patch = read("headless.patch");
    assert!(patch
        .contains("+CONSOLE_TOOLS = {\"eql_headless\", \"eql_quest_cli\", \"eql_fights_cli\"}"));
    assert!(patch.contains("+        console=tool in CONSOLE_TOOLS,"));
    assert!(patch.contains("+HIDDEN = {\"eql_dps_meter\": [\"eql_headless\"]"));
}

/// `CombatTracker.history` is wiped at every session banner and capped at
/// MAX_FIGHT_HISTORY, so reading it instead of listening would silently lose
/// fights.
#[test]
fn the_fights_tool_listens_for_fights_instead_of_reading_the_history() {
    let patch = read("headless.patch");
    assert!(patch.contains("+from eql_combat_tracker import CombatTracker"));
    assert!(patch.contains("+    tracker.fight_listeners.append("));
    assert!(
        !patch.contains("tracker.history"),
        "the fights tool must never read the capped, session-reset history"
    );
    assert!(
        patch.contains(r#"+ZONE_RE = re.compile(TS_RE + r"You have entered (?P<zone>.+)\.\s*$")"#),
        "a fight carries the zone it happened in"
    );
}

/// A tool the patch adds but `apply.sh` does not assert on would go missing on
/// an upstream bump without anything failing.
#[test]
fn apply_sh_insists_on_every_tool_the_daemon_resolves() {
    let script = read("apply.sh");
    for stem in [eqld::tools::HEADLESS_STEM, eqld::tools::FIGHTS_STEM] {
        assert!(
            script.contains(&format!("{stem}.py")),
            "apply.sh drops {stem}"
        );
    }
}

#[test]
fn upstreams_licence_travels_with_the_patch() {
    let licence = read("LICENSE");
    assert!(licence.contains("MIT License"));
    assert!(
        licence.contains("blastlaster"),
        "upstream's copyright line is preserved verbatim"
    );
}
