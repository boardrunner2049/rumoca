//! A Real package constant stays a named constant through the frontend and
//! is inlined only by the `inline-constants` pass.
//!
//! Replacing `P.c` by its value is an optimization, so the frontend declares
//! the constant under its qualified name and keeps the reference (`--pass
//! none` shows it); the `default` group, which a plain compile runs, inlines
//! it and folds the arithmetic it feeds.

use std::fs;
use std::process::{Command, Output};

use tempfile::tempdir;

const FIXTURE: &str = "\
package KC
  package P
    constant Real c = 3.0;
  end P;
  model M
    constant Real k = 2.0;
    Real x(start = 1, fixed = true);
  equation
    der(x) = -k*P.c*x;
  end M;
end KC;
";

fn rumoca(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rumoca"))
        .args(args)
        .output()
        .expect("rumoca runs")
}

fn dae_text(source: &str, passes: &[&str]) -> String {
    model_dae_text(source, "KC.M", passes)
}

fn model_dae_text(source: &str, model: &str, passes: &[&str]) -> String {
    let mut args = vec!["compile", source, "--model", model, "--emit", "dae-mo"];
    for pass in passes {
        args.extend(["--pass", pass]);
    }
    let output = rumoca(&args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn the_frontend_declares_a_package_constant_and_the_pass_inlines_it() {
    let work = tempdir().expect("temp dir");
    let source = work.path().join("KC.mo");
    fs::write(&source, FIXTURE).expect("fixture written");
    let source = source.to_str().expect("utf-8 path");

    let lowered = dae_text(source, &["none"]);
    assert!(
        lowered.contains("constant Real KC.P.c = 3.0"),
        "the package constant is declared: {lowered}"
    );
    assert!(
        lowered.contains("KC.P.c) * x"),
        "the equation still names it: {lowered}"
    );

    let optimized = dae_text(source, &["default"]);
    assert!(
        optimized.contains("6.0 * x"),
        "inline-constants then fold-constants reduce k*P.c to 6: {optimized}"
    );
    assert_eq!(
        dae_text(source, &[]),
        optimized,
        "a plain compile runs the default group"
    );
}

const BINDING_FIXTURE: &str = "\
package KB
  package P
    constant Real c = 3.0;
  end P;
  model M
    parameter Real w = 2*P.c;
    Real x(start = 1, fixed = true);
  equation
    der(x) = -w*x;
  end M;
end KB;
";

#[test]
fn a_binding_keeps_the_package_constant_named_until_the_passes_fold_it() {
    let work = tempdir().expect("temp dir");
    let source = work.path().join("KB.mo");
    fs::write(&source, BINDING_FIXTURE).expect("fixture written");
    let source = source.to_str().expect("utf-8 path");
    let lowered = model_dae_text(source, "KB.M", &["none"]);
    assert!(
        lowered.contains("KB.P.c"),
        "the binding names the declared constant: {lowered}"
    );
    let optimized = model_dae_text(source, "KB.M", &[]);
    assert!(
        optimized.contains("parameter Real w(start = 6.0) = 6.0;"),
        "inline-constants then fold-parameter-bindings give w its value: {optimized}"
    );
}
