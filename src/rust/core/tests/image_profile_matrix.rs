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

fn assert_defaults_supported(profiles: &Value) {
    for (base, profile) in profiles["bases"].as_object().expect("base profiles") {
        let defaults = profile["defaults"].as_object().expect("profile defaults");
        for key in [
            "desktop",
            "editor",
            "dev_runtimes",
            "desktop_apps",
            "pentest",
            "backup_transfer",
        ] {
            let value = defaults[key].as_str().expect("default option string");
            let allowed = profile[key].as_array().expect("profile option array");
            assert!(
                allowed
                    .iter()
                    .any(|candidate| candidate.as_str() == Some(value)),
                "{base} has unsupported default {key}={value}"
            );
        }
    }
}

fn package_prefix(base: &str, options: [&str; 6], profile: &Value) -> String {
    let keys = [
        "desktop",
        "editor",
        "dev_runtimes",
        "desktop_apps",
        "pentest",
        "backup_transfer",
    ];
    let is_default = keys
        .iter()
        .zip(options)
        .all(|(key, value)| profile["defaults"][key].as_str() == Some(value));
    if is_default {
        format!("dev-workspace-{base}-{}", options[0])
    } else {
        format!(
            "dev-workspace-{base}-{}-editor-{}-runtimes-{}-apps-{}-pentest-{}-transfer-{}",
            options[0], options[1], options[2], options[3], options[4], options[5]
        )
    }
}

#[test]
fn every_legal_profile_has_a_unique_readable_package_prefix() {
    let profiles: Value =
        serde_json::from_str(&read_json("profiles.json")).expect("valid profiles");
    let mut names = BTreeMap::<String, String>::new();
    let mut count = 0;
    for (base, profile) in profiles["bases"].as_object().expect("base profiles") {
        let values = |key: &str| {
            profile[key]
                .as_array()
                .expect("profile option array")
                .iter()
                .map(|value| value.as_str().expect("option string"))
                .collect::<Vec<_>>()
        };
        for desktop in values("desktop") {
            for editor in values("editor") {
                for runtimes in values("dev_runtimes") {
                    for apps in values("desktop_apps") {
                        for pentest in values("pentest") {
                            for transfer in values("backup_transfer") {
                                let options = [desktop, editor, runtimes, apps, pentest, transfer];
                                let name = package_prefix(base, options, profile);
                                let description = format!("{base}:{}", options.join("|"));
                                assert!(
                                    names.insert(name.clone(), description.clone()).is_none(),
                                    "package prefix collision for {description}: {name}"
                                );
                                assert!(
                                    name.len() < 200
                                        && name.chars().all(|ch| {
                                            ch.is_ascii_lowercase()
                                                || ch.is_ascii_digit()
                                                || ch == '-'
                                        }),
                                    "invalid package prefix: {name}"
                                );
                                count += 1;
                            }
                        }
                    }
                }
            }
        }
    }
    assert_eq!(count, 257);
    assert_eq!(
        package_prefix(
            "kali",
            [
                "kasmvnc",
                "code-server",
                "standard",
                "firefox",
                "bootstrap",
                "standard"
            ],
            &profiles["bases"]["kali"]
        ),
        "dev-workspace-kali-kasmvnc"
    );
    assert_eq!(
        package_prefix(
            "kali",
            ["kasmvnc", "none", "none", "firefox", "none", "none"],
            &profiles["bases"]["kali"]
        ),
        "dev-workspace-kali-kasmvnc-editor-none-runtimes-none-apps-firefox-pentest-none-transfer-none"
    );
}

#[test]
fn matrix_is_compatible_unique_and_pairwise_complete() {
    let matrix: Matrix =
        serde_json::from_str(&read_json("test-matrix.json")).expect("valid matrix");
    let profiles: Value =
        serde_json::from_str(&read_json("profiles.json")).expect("valid profiles");
    assert_eq!(matrix.schema, 1);
    assert_eq!(matrix.coverage, "pairwise-plus-boundaries");
    assert_defaults_supported(&profiles);

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
