use chadcad_kernel::{EdgeSelector, Format, Side};
use chadcad_model::{AxisKind, Combine, Document, FeatureKind};
use chadcad_timeline::{Engine, export_document};

const CHORD: f64 = 0.1;

fn plate(doc: &mut Document) -> (u64, u64, u64) {
    doc.set_parameter("w", "40".into());
    doc.set_parameter("h", "10".into());
    doc.set_parameter("hole", "w / 8".into());
    let base = doc.add_feature(FeatureKind::Box {
        origin: Default::default(),
        size: ["w".into(), 30.0.into(), "h".into()],
        op: Combine::Join,
    });
    let bore = doc.add_feature(FeatureKind::Cylinder {
        origin: ["w/2".into(), 15.0.into(), (-1.0).into()],
        axis: AxisKind::Z,
        radius: "hole".into(),
        height: "h + 2".into(),
        op: Combine::Cut,
    });
    let round = doc.add_feature(FeatureKind::Chamfer {
        edges: EdgeSelector::OnSide(Side::Top),
        distance: 1.0.into(),
    });
    (base, bore, round)
}

fn volume(r: &chadcad_timeline::BuildResult) -> f64 {
    r.properties.unwrap().volume
}

#[test]
fn builds_and_reports_properties() {
    let mut doc = Document::default();
    plate(&mut doc);
    doc.rollback = Some(2);
    let r = Engine::new().build(&doc, CHORD);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let expected = 40. * 30. * 10. - std::f64::consts::PI * 25. * 10.;
    assert!((volume(&r) - expected).abs() < 1e-3);
    assert!(!r.mesh.indices.is_empty());
}

#[test]
fn second_build_rebuilds_nothing() {
    let mut doc = Document::default();
    plate(&mut doc);
    let mut e = Engine::new();
    assert_eq!(e.build(&doc, CHORD).rebuilt, 3);
    assert_eq!(e.build(&doc, CHORD).rebuilt, 0);
}

#[test]
fn only_features_after_an_edit_rebuild() {
    let mut doc = Document::default();
    let (_, bore, _) = plate(&mut doc);
    let mut e = Engine::new();
    e.build(&doc, CHORD);
    let Some(f) = doc.feature_mut(bore) else { panic!() };
    let FeatureKind::Cylinder { radius, .. } = &mut f.kind else { panic!() };
    *radius = 7.0.into();
    assert_eq!(e.build(&doc, CHORD).rebuilt, 2, "base must come from cache");
}

#[test]
fn parameter_edits_are_tracked_individually() {
    let mut doc = Document::default();
    plate(&mut doc);
    let mut e = Engine::new();
    e.build(&doc, CHORD);

    // Equal value through a different expression: nothing changes downstream.
    doc.set_parameter("h", "5 + 5".into());
    assert_eq!(e.build(&doc, CHORD).rebuilt, 0);

    // `hole` depends on `w` only, and the bore is the first user of `hole`.
    doc.set_parameter("hole", "6".into());
    assert_eq!(e.build(&doc, CHORD).rebuilt, 2);

    // An unused parameter costs nothing.
    doc.set_parameter("spare", "1".into());
    assert_eq!(e.build(&doc, CHORD).rebuilt, 0);
}

#[test]
fn appending_keeps_the_prefix_and_scrubbing_is_cached() {
    let mut doc = Document::default();
    plate(&mut doc);
    let mut e = Engine::new();
    e.build(&doc, CHORD);
    doc.add_feature(FeatureKind::Translate { by: [1.0.into(), 0.0.into(), 0.0.into()] });
    assert_eq!(e.build(&doc, CHORD).rebuilt, 1);
    doc.rollback = Some(1);
    let r = e.build(&doc, CHORD);
    assert_eq!(r.rebuilt, 0);
    assert!((volume(&r) - 12000.).abs() < 1e-6);
}

#[test]
fn suppression_and_errors_keep_the_timeline_going() {
    let mut doc = Document::default();
    let (_, bore, round) = plate(&mut doc);
    doc.feature_mut(bore).unwrap().suppressed = true;
    doc.feature_mut(round).unwrap().suppressed = true;
    let mut e = Engine::new();
    assert!((volume(&e.build(&doc, CHORD)) - 12000.).abs() < 1e-6);

    doc.feature_mut(bore).unwrap().suppressed = false;
    let f = doc.feature_mut(bore).unwrap();
    let FeatureKind::Cylinder { radius, .. } = &mut f.kind else { panic!() };
    *radius = "missing".into();
    let r = e.build(&doc, CHORD);
    assert!(r.errors[&bore].contains("missing"), "{:?}", r.errors);
    assert!((volume(&r) - 12000.).abs() < 1e-6, "failed feature passes the body through");
}

#[test]
fn parameter_cycles_are_errors_not_hangs() {
    let mut doc = Document::default();
    doc.set_parameter("a", "b + 1".into());
    doc.set_parameter("b", "a + 1".into());
    doc.add_feature(FeatureKind::Sphere {
        center: Default::default(),
        radius: "a".into(),
        op: Combine::Join,
    });
    let r = Engine::new().build(&doc, CHORD);
    assert_eq!(r.errors.len(), 1);
    assert!(r.parameters[0].1.is_err());
}

#[test]
fn script_feature_is_parametric() {
    let mut doc = Document::default();
    doc.set_parameter("r", "4".into());
    doc.add_feature(FeatureKind::Script {
        source: "return sphere(params.r)".into(),
        op: Combine::Join,
        replace: false,
    });
    let mut e = Engine::new();
    let v = volume(&e.build(&doc, CHORD));
    assert!((v - 4. / 3. * std::f64::consts::PI * 64.).abs() < 1e-2, "{v}");
    doc.set_parameter("r", "2".into());
    let r = e.build(&doc, CHORD);
    assert_eq!(r.rebuilt, 1);
    assert!(volume(&r) < v);
}

#[test]
fn replacing_script_rewrites_the_body() {
    let mut doc = Document::default();
    doc.add_feature(FeatureKind::Box {
        origin: Default::default(),
        size: [10.0.into(), 10.0.into(), 10.0.into()],
        op: Combine::Join,
    });
    doc.add_feature(FeatureKind::Script {
        source: "return body:fillet(1, 'z')".into(),
        op: Combine::Join,
        replace: true,
    });
    let r = Engine::new().build(&doc, CHORD);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert!(volume(&r) < 1000.);
}

#[test]
fn patterns_multiply_material() {
    let mut doc = Document::default();
    doc.add_feature(FeatureKind::Box {
        origin: Default::default(),
        size: [1.0.into(), 1.0.into(), 1.0.into()],
        op: Combine::Join,
    });
    doc.add_feature(FeatureKind::LinearPattern {
        direction: [1.0.into(), 0.0.into(), 0.0.into()],
        count: 3.0.into(),
        spacing: 2.0.into(),
    });
    let r = Engine::new().build(&doc, CHORD);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert!((volume(&r) - 3.).abs() < 1e-6);
}

#[test]
fn exports_step_and_iges_from_a_document() {
    let mut doc = Document::default();
    plate(&mut doc);
    let step = export_document(&doc, Format::Step, CHORD).unwrap();
    assert!(String::from_utf8_lossy(&step).starts_with("ISO-10303-21"));
    assert!(!export_document(&doc, Format::Iges, CHORD).unwrap().is_empty());
}
