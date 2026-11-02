use rumoca_compile::compile::SourceRootKind;
use rumoca_compile::{Session, SessionConfig};
fn main() {
    let mut session = Session::new(SessionConfig {
        freeze_parameters: true,
        ..SessionConfig::default()
    });
    let parsed = rumoca_compile::source_roots::parse_source_root_with_cache(std::path::Path::new(
        "target/msl/ModelicaStandardLibrary-4.1.0",
    ))
    .expect("MSL");
    session.replace_parsed_source_set("msl", SourceRootKind::External, parsed.documents, None);
    let source = std::fs::read_to_string("examples/modelsan/KnownMSLIssues.mo").unwrap();
    session
        .add_document("examples/modelsan/KnownMSLIssues.mo", &source)
        .unwrap();
    let model = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "KnownMSLIssues.MoistAirFull".to_owned());
    let flat = session
        .compile_model_flat_strict_reachable_uncached_with_recovery(&model)
        .expect("flat");
    std::fs::write(
        "/tmp/modelsan-moistair-triage/full-flat.json",
        serde_json::to_string_pretty(&flat).unwrap(),
    )
    .unwrap();
    println!("flat: {} functions", flat.functions.len());
    let dae = session
        .compile_model_dae_strict_reachable_uncached_with_recovery(&model)
        .expect("dae");
    std::fs::write(
        "/tmp/modelsan-moistair-triage/full-dae-debug.txt",
        format!("{dae:#?}"),
    )
    .unwrap();
    println!("DAE succeeded");
}
