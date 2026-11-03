//! A real ME event/periodic coincidence, in addition to recorder unit tests.
use super::*;
use crate::fmi_me::{
    PublicationObserver,
    driver::{batch_output_cursor, batch_session_options},
};
use std::{cell::RefCell, rc::Rc};

#[test]
fn executable_publication_deduplicates_a_real_scheduled_event() {
    struct Observer(
        Rc<RefCell<Vec<(f64, String)>>>,
        rumoca_eval_solve::execution::CsvExecution,
    );
    impl PublicationObserver for Observer {
        fn publish(
            &mut self,
            t: f64,
            phase: &str,
            names: &[String],
            values: &[f64],
        ) -> Result<(), String> {
            self.0.borrow_mut().push((t, phase.into()));
            self.1.publish(t, phase, names, values)
        }
        fn finish(&mut self) -> Result<(), String> {
            self.1.finish()
        }
    }
    let rows = Rc::new(RefCell::new(Vec::new()));
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("trace");
    let effects = rumoca_eval_solve::execution::CsvExecution::start(
        publication_program(),
        publication_numerical(),
        &root,
    )
    .unwrap();
    let mut model = solve::SolveModel::default();
    model.problem.events.scheduled_time_events = vec![0.5];
    let model = refresh_owned(model);
    let retained = MeRetainedComponent::instantiate(
        MeModelSource::fixture(&model),
        &fixture_instance_config(),
        None,
    )
    .unwrap();
    let options = batch_session_options(
        &rumoca_ir_solve::fmi::RootLocationPlan::STANDARD,
        0.0,
        1.0,
        1e-9,
        1e-11,
        0.1,
        None,
    )
    .unwrap();
    let mut cursor = batch_output_cursor(&options).unwrap();
    let host = retained
        .into_lease_with_observer(options, Some(Box::new(Observer(rows.clone(), effects))))
        .unwrap();
    let mut session = host.into_session(None).unwrap();
    session.run_to_stop(&mut cursor).unwrap();
    session.finish_publication().unwrap();
    let rows = rows.borrow();
    assert_eq!(rows.len(), 11);
    assert_eq!(rows[0], (0.0, "initial".into()));
    let event: Vec<_> = rows
        .iter()
        .filter(|(t, _)| (*t - 0.5).abs() < 1e-12)
        .collect();
    assert_eq!(event.len(), 1);
    assert_eq!(event[0].1, "settled");
    let csv = std::fs::read_to_string(root.join("event.csv")).unwrap();
    assert_eq!(csv.lines().count(), 12);
    assert_eq!(csv.lines().nth(6).unwrap(), "0.5,5,settled");
}

fn publication_program() -> solve::execution::ExecutionArtifact {
    serde_json::from_value(serde_json::json!({
        "version": 2,
        "lowering": "solve-scalar-v1",
        "dependency_digest": "test-event-host",
        "revision": 1,
        "passes": [],
        "program": {
            "sinks": [{
                "key": "event",
                "filename": "event.csv",
                "columns": [
                    {"name": "time", "ty": "real"},
                    {"name": "id", "ty": "integer"},
                    {"name": "phase", "ty": "text"}
                ],
                "metadata": {"connector": {"connector": 0, "orientation": "outside"}, "members": []}
            }],
            "functions": {
                "run_start": {
                    "locals": [],
                    "body": [{"op": "csv.open", "sink": "event"}]
                },
                "publish": {
                    "locals": [
                        {"name": "t", "ty": "real"},
                        {"name": "id", "ty": "integer"},
                        {"name": "phase", "ty": "text"}
                    ],
                    "body": [
                        {"op": "snapshot.time", "result": "t"},
                        {"op": "snapshot.sequence", "result": "id"},
                        {"op": "snapshot.phase", "result": "phase"},
                        {"op": "csv.write_row", "sink": "event", "values": ["t", "id", "phase"]}
                    ]
                },
                "run_finish": {
                    "locals": [],
                    "body": [{"op": "csv.close", "sink": "event"}]
                }
            }
        }
    }))
    .unwrap()
}

/// An empty derived program.
///
/// v2 does not serialize one, so this test builds it directly rather than
/// decoding it from the fixture -- which is what the execution-IR v2 design
/// note said this test should do once the wire stopped carrying it.
fn publication_numerical() -> solve::execution::NumericalProgram {
    solve::execution::NumericalProgram {
        source_name: "event-publication-test".into(),
        storage: vec![],
        residual: vec![],
        derivatives: vec![],
        initialization: vec![],
        algebraic_blocks: vec![],
        initial_blocks: vec![],
        observations: vec![],
    }
}
