use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[test]
fn animatable_reports_focused_shape_policy_and_field_diagnostics() {
    let cases = [
        CompileFailure {
            name: "missing_enum_policy",
            source: r#"
use aimer_animation::Animatable;

#[derive(Animatable)]
enum State {
    Idle,
    Running,
}
"#,
            diagnostic: "Animatable enums require `#[animatable(discrete)]` or `#[animatable(fieldwise)]`",
        },
        CompileFailure {
            name: "duplicate_enum_policy",
            source: r#"
use aimer_animation::Animatable;

#[derive(Animatable)]
#[animatable(discrete, fieldwise)]
enum State {
    Idle,
    Running,
}
"#,
            diagnostic: "Animatable enum policy may only be specified once",
        },
        CompileFailure {
            name: "unknown_enum_policy",
            source: r#"
use aimer_animation::Animatable;

#[derive(Animatable)]
#[animatable(custom)]
enum State {
    Idle,
    Running,
}
"#,
            diagnostic: "unsupported Animatable policy; expected `discrete` or `fieldwise`",
        },
        CompileFailure {
            name: "valued_enum_policy",
            source: r#"
use aimer_animation::Animatable;

#[derive(Animatable)]
#[animatable(discrete = true)]
enum State {
    Idle,
    Running,
}
"#,
            diagnostic: "Animatable policies do not accept values",
        },
        CompileFailure {
            name: "policy_on_struct",
            source: r#"
use aimer_animation::Animatable;

#[derive(Animatable)]
#[animatable(discrete)]
struct Value {
    amount: f32,
}
"#,
            diagnostic: "Animatable policies are only valid on enums",
        },
        CompileFailure {
            name: "policy_on_variant",
            source: r#"
use aimer_animation::Animatable;

#[derive(Animatable)]
#[animatable(discrete)]
enum State {
    #[animatable(fieldwise)]
    Idle,
    Running,
}
"#,
            diagnostic: "Animatable policy must be placed on the enum definition",
        },
        CompileFailure {
            name: "union",
            source: r#"
use aimer_animation::Animatable;

#[derive(Animatable)]
union Value {
    amount: f32,
}
"#,
            diagnostic: "Animatable cannot be derived for unions",
        },
        CompileFailure {
            name: "empty_enum",
            source: r#"
use aimer_animation::Animatable;

#[derive(Animatable)]
#[animatable(discrete)]
enum State {}
"#,
            diagnostic: "Animatable cannot be derived for an enum with no variants",
        },
        CompileFailure {
            name: "unsupported_string_field",
            source: r#"
use aimer_animation::Animatable;

#[derive(Animatable)]
struct Label {
    text: String,
}
"#,
            diagnostic: "String: Animatable",
        },
        CompileFailure {
            name: "unsupported_option_field",
            source: r#"
use aimer_animation::Animatable;

#[derive(Animatable)]
struct OptionalValue {
    value: Option<f32>,
}
"#,
            diagnostic: "Option<f32>: Animatable",
        },
        CompileFailure {
            name: "unsupported_bool_field",
            source: r#"
use aimer_animation::Animatable;

#[derive(Animatable)]
struct Toggle {
    enabled: bool,
}
"#,
            diagnostic: "bool: Animatable",
        },
    ];

    for case in cases {
        let output = check_case(case.name, case.source);
        assert_failure(&output, case.diagnostic);
    }
}

struct CompileFailure {
    name: &'static str,
    source: &'static str,
    diagnostic: &'static str,
}

fn check_case(name: &str, source: &str) -> Output {
    let fixture = fixture_root().join(name);
    if fixture.exists() {
        fs::remove_dir_all(&fixture).unwrap();
    }
    fs::create_dir_all(fixture.join("src")).unwrap();
    fs::write(fixture.join("Cargo.toml"), fixture_manifest(name)).unwrap();
    fs::write(fixture.join("src/lib.rs"), source).unwrap();

    Command::new(env!("CARGO"))
        .args(["check", "--quiet", "--offline", "--manifest-path"])
        .arg(fixture.join("Cargo.toml"))
        .env("CARGO_TARGET_DIR", fixture_root().join("target"))
        .output()
        .unwrap()
}

fn fixture_manifest(name: &str) -> String {
    let macro_crate = Path::new(env!("CARGO_MANIFEST_DIR"));
    let animation_crate = macro_crate.join("../aimer_animation");
    format!(
        r#"[package]
name = "aimer_macro_animatable_{name}"
version = "0.0.0"
edition = "2024"

[workspace]

[dependencies]
aimer_animation = {{ path = {:?} }}
"#,
        animation_crate,
    )
}

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/aimer_macro_animatable_compile")
}

fn assert_failure(output: &Output, diagnostic: &str) {
    assert!(!output.status.success(), "fixture unexpectedly compiled");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(diagnostic),
        "expected diagnostic `{diagnostic}` but compiler reported:\n{stderr}"
    );
}
