use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct Case {
    id: String,
    name: String,
    base: String,
    desktop: String,
    editor: String,
    dev_runtimes: String,
    desktop_apps: String,
    pentest: String,
    backup_transfer: String,
}

#[derive(Debug, Deserialize)]
struct Matrix {
    schema: u64,
    coverage: String,
    expected_pair_count: usize,
    cases: Vec<Case>,
}

fn image_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../image")
}

fn read_json(name: &str) -> String {
    fs::read_to_string(image_dir().join(name)).expect("read image profile data")
}

fn values(case: &Case) -> [&str; 7] {
    [
        &case.base,
        &case.desktop,
        &case.editor,
        &case.dev_runtimes,
        &case.desktop_apps,
        &case.pentest,
        &case.backup_transfer,
    ]
}

fn pairs(row: &[&str; 7]) -> BTreeSet<(usize, String, usize, String)> {
    let mut out = BTreeSet::new();
    for left in 0..row.len() {
        for right in left + 1..row.len() {
            out.insert((left, row[left].to_string(), right, row[right].to_string()));
        }
    }
    out
}

fn assert_case_supported(case: &Case, profiles: &Value) {
    let profile = &profiles["bases"][&case.base];
    assert!(
        !profile.is_null(),
        "{} has unknown base {}",
        case.id,
        case.base
    );
    for (key, value) in [
        ("desktop", &case.desktop),
        ("editor", &case.editor),
        ("dev_runtimes", &case.dev_runtimes),
        ("desktop_apps", &case.desktop_apps),
        ("pentest", &case.pentest),
        ("backup_transfer", &case.backup_transfer),
    ] {
        let allowed = profile[key].as_array().expect("profile option array");
        assert!(
            allowed
                .iter()
                .any(|candidate| candidate.as_str() == Some(value)),
            "{} requests unsupported {}={}",
            case.id,
            key,
            value
        );
    }
}

#[test]
fn matrix_is_compatible_unique_and_pairwise_complete() {
    let matrix: Matrix =
        serde_json::from_str(&read_json("test-matrix.json")).expect("valid matrix");
    let profiles: Value =
        serde_json::from_str(&read_json("profiles.json")).expect("valid profiles");
    assert_eq!(matrix.schema, 1);
    assert_eq!(matrix.coverage, "pairwise-plus-boundaries");

    let ids: BTreeSet<_> = matrix.cases.iter().map(|case| &case.id).collect();
    assert_eq!(ids.len(), matrix.cases.len(), "case ids must be unique");
    for case in &matrix.cases {
        assert!(
            !case.name.trim().is_empty(),
            "{} needs a readable name",
            case.id
        );
        assert_case_supported(case, &profiles);
    }

    let by_id: BTreeMap<_, _> = matrix
        .cases
        .iter()
        .map(|case| (case.id.as_str(), case))
        .collect();
    assert_eq!(
        values(by_id["p01"]),
        [
            "kali",
            "kasmvnc",
            "code-server",
            "standard",
            "firefox",
            "bootstrap",
            "standard"
        ]
    );
    assert_eq!(
        values(by_id["p02"]),
        [
            "debian",
            "kasmvnc",
            "code-server",
            "standard",
            "firefox",
            "bootstrap",
            "standard"
        ]
    );
    assert_eq!(
        values(by_id["p03"]),
        ["kali", "none", "none", "none", "none", "none", "none"]
    );
    assert_eq!(
        values(by_id["p04"]),
        ["debian", "none", "none", "none", "none", "none", "none"]
    );
    assert_eq!(
        values(by_id["p05"]),
        [
            "kali",
            "both",
            "code-server",
            "standard",
            "firefox",
            "bootstrap",
            "standard"
        ]
    );
    assert_eq!(
        values(by_id["p13"]),
        ["alpine", "none", "none", "none", "none", "none", "none"]
    );

    let apt_cases: Vec<_> = matrix
        .cases
        .iter()
        .filter(|case| case.base == "kali" || case.base == "debian")
        .collect();
    let mut covered = BTreeSet::new();
    for case in apt_cases {
        covered.extend(pairs(&values(case)));
    }

    let levels = [
        vec!["kali", "debian"],
        vec!["none", "kasmvnc", "selkies", "both"],
        vec!["none", "code-server"],
        vec!["none", "standard"],
        vec!["none", "firefox"],
        vec!["none", "bootstrap"],
        vec!["none", "standard"],
    ];
    let mut required = BTreeSet::new();
    for left in 0..levels.len() {
        for right in left + 1..levels.len() {
            for left_value in &levels[left] {
                for right_value in &levels[right] {
                    required.insert((
                        left,
                        (*left_value).to_string(),
                        right,
                        (*right_value).to_string(),
                    ));
                }
            }
        }
    }
    let missing: Vec<_> = required.difference(&covered).cloned().collect();
    assert!(
        missing.is_empty(),
        "matrix misses pairwise cases: {missing:?}"
    );
    assert_eq!(required.len(), matrix.expected_pair_count);
}
