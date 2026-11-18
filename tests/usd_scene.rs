use clothcraft::{
    garment::GarmentTemplate,
    usd_scene::{
        ShadingMode, append_viewport_surface, display_normals, flat_corner_normals,
        garment_stage_usda, smooth_vertex_normals,
    },
};
use glam::DVec3;

fn approx(a: DVec3, b: DVec3) -> bool {
    (a - b).length() < 1e-12
}

fn tiny() -> clothcraft::garment::CompiledGarment {
    let mut template = GarmentTemplate::two_panel_demo();
    for panel in &mut template.panels {
        panel.pattern.nx = 1;
        panel.pattern.ny = 1;
    }
    template.compile().unwrap()
}

#[test]
fn scene_publishes_local_topology_and_preserves_winding() {
    let garment = tiny();
    let text = garment_stage_usda(&garment, &garment.initial_state).unwrap();
    assert!(text.starts_with("#usda 1.0\n"));
    assert!(text.contains("metersPerUnit = 1"));
    assert!(text.contains("upAxis = \"Y\""));
    assert_eq!(text.matches("faceVertexCounts = [3, 3]").count(), 2);
    // Independently known connectivity for a one-cell panel; the second panel
    // must restart at zero instead of retaining global mesh offsets.
    assert_eq!(
        text.matches("faceVertexIndices = [0, 1, 3, 0, 3, 2]")
            .count(),
        2
    );
    assert_eq!(text.matches("subdivisionScheme = \"none\"").count(), 2);
    assert_eq!(text.matches("orientation = \"rightHanded\"").count(), 2);
    assert_eq!(text.matches("doubleSided = true").count(), 2);
}

#[test]
fn deformation_changes_points_without_changing_material_coordinates() {
    let garment = tiny();
    let before = garment_stage_usda(&garment, &garment.initial_state).unwrap();
    let mut state = garment.initial_state.clone();
    state.x[0] += DVec3::new(0.125, -0.25, 0.5);
    state.time = 1.25;
    let after = garment_stage_usda(&garment, &state).unwrap();
    let rest_lines = |s: &str| {
        s.lines()
            .filter(|line| line.contains("restUV ="))
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    assert_eq!(rest_lines(&before), rest_lines(&after));
    assert_ne!(before, after);
    assert!(after.contains("clothcraft:timeSeconds = 1.25"));
    assert_eq!(garment.initial_state.time, 0.0);
}

#[test]
fn rest_coordinates_keep_more_precision_than_render_points() {
    let mut garment = tiny();
    garment.mesh.uv[0].x = 0.123456789012345;
    let text = garment_stage_usda(&garment, &garment.initial_state).unwrap();
    assert!(text.contains("custom double2[] clothcraft:restUV = [(0.123456789012345, 0)"));
    assert!(text.contains("texCoord2f[] primvars:st = [(0.12345679, 0)"));
}

#[test]
fn semantic_ids_are_injective_escaped_and_independent_of_panel_order() {
    let mut garment = tiny();
    garment.panels[0].id = "a-b".into();
    garment.panels[1].id = "a_b".into();
    let text = garment_stage_usda(&garment, &garment.initial_state).unwrap();
    assert!(text.contains("def Mesh \"panel_612d62\""));
    assert!(text.contains("def Mesh \"panel_615f62\""));
    garment.panels.reverse();
    let reordered = garment_stage_usda(&garment, &garment.initial_state).unwrap();
    for name in ["panel_612d62", "panel_615f62"] {
        assert!(reordered.contains(name));
    }
    garment.panels[0].id = "quoted\"\\\n".into();
    let text = garment_stage_usda(&garment, &garment.initial_state).unwrap();
    assert!(text.contains("clothcraft:panelId = \"quoted\\\"\\\\\\n\""));
    garment.panels[0].id = garment.panels[1].id.clone();
    assert!(garment_stage_usda(&garment, &garment.initial_state).is_err());
}

#[test]
fn malformed_partitions_and_cross_panel_triangles_are_rejected() {
    let mut garment = tiny();
    let initial = garment.clone();
    garment.panels[1].vertices = garment.panels[0].vertices.clone();
    assert!(garment_stage_usda(&garment, &garment.initial_state).is_err());
    garment = initial.clone();
    garment.panels[0].triangles.end -= 1;
    assert!(garment_stage_usda(&garment, &garment.initial_state).is_err());
    garment = initial.clone();
    garment.mesh.triangles[0].vertices[0] = garment.panels[1].vertices.start;
    assert!(garment_stage_usda(&garment, &garment.initial_state).is_err());
    garment = initial;
    garment.panels[0].vertices.end = usize::MAX;
    assert!(garment_stage_usda(&garment, &garment.initial_state).is_err());
}

#[test]
fn nonfinite_and_float_overflow_snapshots_are_rejected() {
    let garment = tiny();
    for value in [f64::NAN, f64::INFINITY, f64::MAX] {
        let mut state = garment.initial_state.clone();
        state.x[0].x = value;
        assert!(garment_stage_usda(&garment, &state).is_err());
    }
    let mut state = garment.initial_state.clone();
    state.x.pop();
    assert!(garment_stage_usda(&garment, &state).is_err());
    state = garment.initial_state.clone();
    state.time = f64::NAN;
    assert!(garment_stage_usda(&garment, &state).is_err());
    let mut garment = garment;
    garment.mesh.uv[0].y = f64::NAN;
    assert!(garment_stage_usda(&garment, &garment.initial_state).is_err());
}

#[test]
fn mesh_stage_usda_publishes_model_mesh() {
    let project = clothcraft::project::Project::default();
    let (mesh, state) = project.build().unwrap();
    let usda = clothcraft::usd_scene::mesh_stage_usda(&mesh, &state).unwrap();
    assert!(usda.starts_with("#usda 1.0\n"));
    assert!(usda.contains("def Mesh \"cloth_mesh\""));
    assert!(usda.contains("custom string clothcraft:panelId = \"cloth\""));
    assert!(usda.contains("point3f[] points = ["));
    assert!(usda.contains("int[] faceVertexCounts = ["));
    assert!(usda.contains("int[] faceVertexIndices = ["));
    assert!(usda.contains("normal3f[] normals = ["));
}

#[test]
fn smooth_normals_average_incident_faces_and_flat_normals_stay_per_face() {
    // Two triangles share edge 0-1 and fold by 90 degrees: one faces +Z, one +Y.
    let points = [DVec3::ZERO, DVec3::X, DVec3::Y, DVec3::Z];
    let triangles = [[0, 1, 2], [0, 3, 1]];
    let smooth = smooth_vertex_normals(&points, &triangles);
    let shared = DVec3::new(0.0, 1.0, 1.0).normalize();
    assert!(approx(smooth[0], shared), "{:?}", smooth[0]);
    assert!(approx(smooth[1], shared), "{:?}", smooth[1]);
    assert!(approx(smooth[2], DVec3::Z), "{:?}", smooth[2]);
    assert!(approx(smooth[3], DVec3::Y), "{:?}", smooth[3]);

    let flat = flat_corner_normals(&points, &triangles);
    assert_eq!(flat.len(), 6);
    assert!(flat[0..3].iter().all(|n| approx(*n, DVec3::Z)));
    assert!(flat[3..6].iter().all(|n| approx(*n, DVec3::Y)));

    assert_eq!(
        display_normals(&points, &triangles, ShadingMode::Smooth),
        smooth
    );
    assert_eq!(
        display_normals(&points, &triangles, ShadingMode::Flat),
        flat
    );
}

#[test]
fn degenerate_triangles_and_out_of_range_indices_get_finite_fallback_normals() {
    let points = [DVec3::ZERO; 3];
    for normals in [
        smooth_vertex_normals(&points, &[[0, 1, 2]]),
        flat_corner_normals(&points, &[[0, 1, 2]]),
        smooth_vertex_normals(&points[..1], &[[0, 1, 2]]),
    ] {
        assert!(!normals.is_empty());
        assert!(
            normals
                .iter()
                .all(|n| n.is_finite() && approx(*n, DVec3::Y)),
            "{normals:?}"
        );
    }
}

#[test]
fn scene_publishes_smooth_vertex_normals_for_every_panel() {
    let garment = tiny();
    let text = garment_stage_usda(&garment, &garment.initial_state).unwrap();
    assert_eq!(text.matches("normal3f[] normals = [").count(), 2);
    assert_eq!(
        text.matches("interpolation = \"vertex\"\n            )\n            custom double2[]")
            .count(),
        2
    );
    // One normals value per panel vertex (a 1x1 panel publishes four points).
    for line in text
        .lines()
        .filter(|line| line.trim_start().starts_with("normal3f[] normals ="))
    {
        let values = line.split_once("= [").unwrap().1;
        let values = values.split_once("] (").unwrap().0;
        assert_eq!(values.matches('(').count(), 4, "{line}");
    }
}

#[test]
fn collision_display_surface_publishes_smooth_normals() {
    let surface = clothcraft::avatar::SurfaceMesh {
        vertices: vec![DVec3::ZERO, DVec3::X, DVec3::Y, DVec3::Z],
        triangles: vec![[0, 1, 2], [0, 3, 1]],
    };
    let usda = append_viewport_surface(String::new(), &surface);
    assert_eq!(usda.matches("normal3f[] normals = [").count(), 1);
    assert_eq!(usda.matches("interpolation = \"vertex\"\n )\n").count(), 1);
}
