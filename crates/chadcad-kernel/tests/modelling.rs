use chadcad_kernel::*;

fn close(a: f64, b: f64, tol: f64) {
    assert!((a - b).abs() < tol, "{a} vs {b}");
}

/// A 40x30x10 plate with a through hole.
fn plate() -> (Bench, Handle) {
    let mut b = Bench::new();
    let plate = b.cuboid([0., 0., 0.], [40., 30., 10.]).unwrap();
    let hole = b.cylinder([20., 15., -1.], [0., 0., 1.], 5., 12.).unwrap();
    let cut = b.cut(&plate, &hole).unwrap();
    (b, cut)
}

#[test]
fn boolean_volume_is_exact() {
    let (b, s) = plate();
    let p = b.properties(&s).unwrap();
    close(p.volume, 40. * 30. * 10. - std::f64::consts::PI * 250., 1e-3);
    assert!(b.check(&s).unwrap().is_empty());
}

#[test]
fn profile_with_hole_extrudes() {
    let mut b = Bench::new();
    let outer = Loop::Polygon(vec![[0., 0.], [20., 0.], [20., 10.], [0., 10.]]);
    let hole = Loop::Circle { center: [10., 5.], radius: 2. };
    let f = b.profile(PlaneKind::XY, 0., &outer, &[hole]).unwrap();
    let s = b.extrude(&f, [0., 0., 5.], 0.).unwrap();
    close(b.properties(&s).unwrap().volume, 1000. - std::f64::consts::PI * 20., 1e-3);
}

#[test]
fn revolve_makes_a_ring() {
    let mut b = Bench::new();
    let square = Loop::Polygon(vec![[10., 0.], [12., 0.], [12., 2.], [10., 2.]]);
    let f = b.profile(PlaneKind::XZ, 0., &square, &[]).unwrap();
    let s = b.revolve(&f, [0., 0., 0.], [0., 0., 1.], 360.).unwrap();
    close(b.properties(&s).unwrap().volume, 4. * std::f64::consts::TAU * 11., 1e-3);
}

#[test]
fn fillet_and_chamfer_remove_material() {
    let mut b = Bench::new();
    let c = b.cuboid([0., 0., 0.], [20., 20., 20.]).unwrap();
    let f = b.fillet(&c, &EdgeSelector::Parallel(2), 2.).unwrap();
    // Each of 4 corners loses (1 - pi/4) r^2 over the height.
    close(
        b.properties(&f).unwrap().volume,
        8000. - 4. * (1. - std::f64::consts::FRAC_PI_4) * 4. * 20.,
        1e-3,
    );
    let g = b.chamfer(&c, &EdgeSelector::OnSide(Side::Top), 1.).unwrap();
    assert!(b.properties(&g).unwrap().volume < 8000.);
}

#[test]
fn shell_hollows_with_open_top() {
    let mut b = Bench::new();
    let c = b.cuboid([0., 0., 0.], [20., 20., 20.]).unwrap();
    let s = b.shell(&c, &FaceSelector::OnSide(Side::Top), 2.).unwrap();
    close(b.properties(&s).unwrap().volume, 8000. - 16. * 16. * 18., 1e-3);
}

#[test]
fn tessellation_is_in_world_coordinates() {
    let mut b = Bench::new();
    let c = b.cuboid([0., 0., 0.], [1., 1., 1.]).unwrap();
    let moved = b.translate(&c, [100., 0., 0.]).unwrap();
    let m = b.tessellate(&moved, 0.1).unwrap();
    assert_eq!(m.edges.len(), 12);
    assert!(m.positions.iter().all(|p| p[0] >= 99.9));
    assert_eq!(m.indices.len(), 36);
    assert_eq!(m.positions.len(), m.normals.len());
}

#[test]
fn every_format_exports_and_exact_ones_read_back() {
    let (b, s) = plate();
    let volume = b.properties(&s).unwrap().volume;
    for format in Format::ALL {
        let bytes =
            b.export(&s, "plate", format, 0.1).unwrap_or_else(|e| panic!("{format:?}: {e}"));
        assert!(!bytes.is_empty(), "{format:?}");
        if matches!(format, Format::Step | Format::Iges | Format::Brep) {
            close(read_back_volume(&bytes, format).unwrap(), volume, 1e-2);
        }
    }
}

#[test]
fn snapshot_is_send_and_sync() {
    fn check<T: Send + Sync>() {}
    check::<Snapshot>();
}
