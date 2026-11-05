//! Members of a replaceable model alias's selected class (MLS 3.7 §7.3, §5.3).
//!
//! `SensorSuite` declares `replaceable model ImuModel = Sensor constrainedby
//! PartialSensor; ImuModel imu;`. A vehicle modifies a nested suite with
//! `redeclare model ImuModel = DigitalSensor` and reads `plant.sensors.imu.
//! sampleTime`, a member `DigitalSensor` inherits through a three-level
//! `extends` chain and the constraining class lacks. The redeclaration is not
//! itself `replaceable`, so lookup of the member happens in the instance of the
//! selected class (MLS §7.3: a redeclaration replaces the class of the
//! element); the constraining interface only restricts what further
//! redeclarations may select (MLS §7.3.2). A redeclaration written in a nested
//! modifier selects the class exactly as one on the component's own
//! declaration does (MLS §7.2). A member absent from the selected class is
//! still a redeclaration error.

use rumoca::Compiler;

const SOURCE: &str = r#"
package Lib
  partial model PartialSensor
    output Real y;
  end PartialSensor;
  model Sensor
    extends PartialSensor;
  equation
    y = 1;
  end Sensor;
  partial model PartialSampledSensor
    extends PartialSensor;
    output Real sampleTime;
  end PartialSampledSensor;
  partial model PartialDeviceSensor
    extends PartialSampledSensor;
    parameter Real period = 0.5;
  end PartialDeviceSensor;
  model DigitalSensor
    extends PartialDeviceSensor;
  equation
    y = 2;
    sampleTime = period;
  end DigitalSensor;
  model SensorSuite
    replaceable model ImuModel = Sensor constrainedby PartialSensor;
    ImuModel imu;
  end SensorSuite;
  model Vehicle
    SensorSuite sensors;
  end Vehicle;
end Lib;

model Configured
  Lib.Vehicle plant(sensors(redeclare model ImuModel = Lib.DigitalSensor, imu(period = 0.25)));
  Lib.Vehicle nominal;
  Lib.SensorSuite direct(redeclare model ImuModel = Lib.DigitalSensor);
  output Real imuSampleTime;
  output Real directSampleTime;
  output Real imuY;
  output Real nominalY;
equation
  imuSampleTime = plant.sensors.imu.sampleTime;
  directSampleTime = direct.imu.sampleTime;
  imuY = plant.sensors.imu.y;
  nominalY = nominal.sensors.imu.y;
end Configured;

model Absent
  Lib.Vehicle plant(sensors(redeclare model ImuModel = Lib.DigitalSensor));
  output Real missing;
equation
  missing = plant.sensors.imu.noSuchMember;
end Absent;
"#;

#[test]
fn inherited_member_of_selected_alias_class_is_visible_from_outside() {
    let compiled = Compiler::new()
        .model("Configured")
        .compile_str(SOURCE, "ReplaceableModelAliasSelectedMembers.mo")
        .unwrap_or_else(|error| panic!("Configured compiles: {error:?}"));
    let result = rumoca_sim::simulate_dae_with_diagnostics(
        &compiled.dae,
        &rumoca_sim::SimOptions {
            t_end: 0.1,
            ..Default::default()
        },
    )
    .unwrap_or_else(|error| panic!("Configured simulates: {error:?}"));
    let value = |name: &str| {
        let column = result
            .names
            .iter()
            .position(|candidate| candidate == name)
            .unwrap_or_else(|| panic!("{name} in {:?}", result.names));
        result.data[column][0]
    };
    assert_eq!(value("imuSampleTime"), 0.25);
    assert_eq!(value("directSampleTime"), 0.5);
    // The nested redeclaration selects `DigitalSensor` for `plant` only.
    assert_eq!(value("imuY"), 2.0);
    assert_eq!(value("nominalY"), 1.0);
}

#[test]
fn member_absent_from_selected_alias_class_is_a_redeclaration_error() {
    let error = Compiler::new()
        .model("Absent")
        .compile_str(SOURCE, "ReplaceableModelAliasSelectedMembers.mo")
        .expect_err("a member absent from the selected class must not compile");
    let rendered = format!("{error:?}");
    assert!(
        rendered.contains("EI007") && rendered.contains("noSuchMember"),
        "expected EI007 for `noSuchMember`, got {rendered}"
    );
}
