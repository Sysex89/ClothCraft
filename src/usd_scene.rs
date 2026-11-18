//! Static USD publication, independent of the desktop and the physical solvers.
//!
//! This bounded USDA writer is a publication boundary, not a USD parser or stage
//! implementation. Editable templates remain the source of semantic references;
//! the future event-thread scene adapter can load the result into cssimpler's
//! native `openusd::usd::Stage`. See `docs/usd-scene.md` for losses and units.
use crate::{garment::CompiledGarment, model::State};
use std::{collections::BTreeSet, fmt::Write as _};

/// How the viewport derives shading normals from the displayed triangle mesh.
///
/// Both modes publish explicit USD normals instead of leaving the renderer to
/// invent per-face normals. [`ShadingMode::Smooth`] authors one area-weighted
/// normal per point with `vertex` interpolation, so the surface shades
/// continuously across triangle edges. [`ShadingMode::Flat`] authors one normal
/// per triangle corner with `faceVarying` interpolation, so every face keeps a
/// constant normal.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShadingMode {
    #[default]
    Smooth,
    Flat,
}

impl ShadingMode {
    /// USD `normals` interpolation token used by this mode.
    pub fn interpolation(self) -> &'static str {
        match self {
            Self::Smooth => "vertex",
            Self::Flat => "faceVarying",
        }
    }

    /// Short label for the viewport control.
    pub fn label(self) -> &'static str {
        match self {
            Self::Smooth => "Smooth",
            Self::Flat => "Flat",
        }
    }
}

/// Normalized face normal for one triangle, or `+Y` when it is degenerate.
fn face_normal(points: &[glam::DVec3], triangle: [usize; 3]) -> glam::DVec3 {
    let (Some(a), Some(b), Some(c)) = (
        points.get(triangle[0]),
        points.get(triangle[1]),
        points.get(triangle[2]),
    ) else {
        return glam::DVec3::Y;
    };
    let normal = (*b - *a).cross(*c - *a);
    if normal.is_finite() && normal.length_squared() > 0.0 {
        normal.normalize()
    } else {
        glam::DVec3::Y
    }
}

/// Area-weighted per-vertex normals for a triangle mesh.
///
/// Triangle winding defines orientation, matching the renderer's own face
/// normal. Degenerate triangles contribute nothing. A point with no
/// nondegenerate incident triangle, or with non-finite coordinates, receives a
/// finite upward fallback because the viewport rejects zero-length normals.
pub fn smooth_vertex_normals(points: &[glam::DVec3], triangles: &[[usize; 3]]) -> Vec<glam::DVec3> {
    let mut normals = vec![glam::DVec3::ZERO; points.len()];
    for &triangle in triangles {
        let (Some(a), Some(b), Some(c)) = (
            points.get(triangle[0]),
            points.get(triangle[1]),
            points.get(triangle[2]),
        ) else {
            continue;
        };
        let normal = (*b - *a).cross(*c - *a);
        if !normal.is_finite() {
            continue;
        }
        for index in triangle {
            normals[index] += normal;
        }
    }
    for normal in &mut normals {
        *normal = if normal.is_finite() && normal.length_squared() > 0.0 {
            normal.normalize()
        } else {
            glam::DVec3::Y
        };
    }
    normals
}

/// One flat normal per triangle corner, in `faceVertexIndices` order.
pub fn flat_corner_normals(points: &[glam::DVec3], triangles: &[[usize; 3]]) -> Vec<glam::DVec3> {
    let mut normals = Vec::with_capacity(triangles.len() * 3);
    for &triangle in triangles {
        let normal = face_normal(points, triangle);
        normals.extend([normal; 3]);
    }
    normals
}

/// Display normals for a mesh under `mode`, in the matching USD value order.
pub fn display_normals(
    points: &[glam::DVec3],
    triangles: &[[usize; 3]],
    mode: ShadingMode,
) -> Vec<glam::DVec3> {
    match mode {
        ShadingMode::Smooth => smooth_vertex_normals(points, triangles),
        ShadingMode::Flat => flat_corner_normals(points, triangles),
    }
}

/// Append an authored `normal3f[] normals` attribute at `indent`.
fn push_normals(
    out: &mut String,
    indent: &str,
    points: &[glam::DVec3],
    triangles: &[[usize; 3]],
    mode: ShadingMode,
) {
    let normals = display_normals(points, triangles, mode);
    write!(out, "\n{indent}normal3f[] normals = [").unwrap();
    for (j, normal) in normals.iter().enumerate() {
        if j > 0 {
            out.push_str(", ");
        }
        write!(
            out,
            "({}, {}, {})",
            normal.x as f32, normal.y as f32, normal.z as f32
        )
        .unwrap();
    }
    write!(
        out,
        "] (\n{indent}    interpolation = \"{}\"\n{indent})",
        mode.interpolation()
    )
    .unwrap();
}

fn quoted(value: &str) -> Result<String, String> {
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => return Err("USD panel ID contains a control character".into()),
            c => out.push(c),
        }
    }
    out.push('"');
    Ok(out)
}

/// Stable, injective encoding; no sanitization collisions or panel-order indices.
fn prim_name(id: &str) -> String {
    let mut name = String::from("panel_");
    for byte in id.bytes() {
        write!(name, "{byte:02x}").unwrap();
    }
    name
}

fn finite_float(value: f64) -> bool {
    value.is_finite() && (value as f32).is_finite()
}

/// Publish one world-space snapshot as metre-scale, Y-up USDA polygon meshes.
///
/// Each panel becomes a separate Mesh. `points` and `primvars:st` use USD's
/// standard f32 types; `clothcraft:restUV` retains the original f64 material
/// coordinates in metres. Indices are local transient mesh topology, never
/// persistent authoring references. Winding is preserved with `rightHanded`
/// orientation, two-sided cloth, and explicitly disabled subdivision.
///
/// Rejects inconsistent panel partitions, out-of-panel triangles, duplicate IDs,
/// non-finite data and values outside f32 range before returning any artifact.
/// No solver, filesystem, GPU, material network or editable-document state is
/// changed. Velocities, seams, physical properties and other authoring data are
/// deliberately omitted; this is not a project backup or an import adapter.
pub fn garment_stage_usda(garment: &CompiledGarment, state: &State) -> Result<String, String> {
    let mesh = &garment.mesh;
    if state.x.len() != mesh.uv.len() || !state.time.is_finite() {
        return Err("USD snapshot requires matching positions and finite time".into());
    }
    if garment.panels.is_empty() {
        return Err("USD snapshot requires at least one panel".into());
    }
    if state
        .x
        .iter()
        .any(|p| p.to_array().into_iter().any(|v| !finite_float(v)))
        || mesh
            .uv
            .iter()
            .any(|p| p.to_array().into_iter().any(|v| !finite_float(v)))
    {
        return Err("USD points and UVs must be finite and representable as f32".into());
    }
    let mut vertices_seen = vec![false; mesh.uv.len()];
    let mut triangles_seen = vec![false; mesh.triangles.len()];
    let mut ids = BTreeSet::new();
    for panel in &garment.panels {
        if panel.id.is_empty() || !ids.insert(&panel.id) {
            return Err("USD snapshot requires unique nonempty panel IDs".into());
        }
        quoted(&panel.id)?;
        if panel.vertices.start >= panel.vertices.end
            || panel.vertices.end > mesh.uv.len()
            || panel.triangles.start >= panel.triangles.end
            || panel.triangles.end > mesh.triangles.len()
            || panel.vertices.len() > i32::MAX as usize
        {
            return Err(format!("USD panel {} has invalid mesh ranges", panel.id));
        }
        for i in panel.vertices.clone() {
            if std::mem::replace(&mut vertices_seen[i], true) {
                return Err("USD panel vertex ranges overlap".into());
            }
        }
        for i in panel.triangles.clone() {
            if std::mem::replace(&mut triangles_seen[i], true) {
                return Err("USD panel triangle ranges overlap".into());
            }
            let triangle = &mesh.triangles[i];
            if triangle
                .vertices
                .iter()
                .any(|v| !panel.vertices.contains(v))
            {
                return Err(format!(
                    "USD panel {} has an out-of-panel triangle",
                    panel.id
                ));
            }
            let [a, b, c] = triangle.vertices;
            if a == b || b == c || c == a {
                return Err("USD triangle repeats a point index".into());
            }
        }
    }
    if vertices_seen.contains(&false) || triangles_seen.contains(&false) {
        return Err("USD panel ranges must cover the entire mesh".into());
    }

    let mut out = String::from(
        "#usda 1.0\n(\n    defaultPrim = \"World\"\n    metersPerUnit = 1\n    upAxis = \"Y\"\n)\n\ndef Xform \"World\"\n{\n    def Scope \"Garment\"\n    {\n        custom int clothcraft:snapshotVersion = 1\n",
    );
    writeln!(
        out,
        "        custom double clothcraft:timeSeconds = {}",
        state.time
    )
    .unwrap();
    for panel in &garment.panels {
        writeln!(
            out,
            "        def Mesh \"{}\"\n        {{",
            prim_name(&panel.id)
        )
        .unwrap();
        writeln!(
            out,
            "            custom string clothcraft:panelId = {}",
            quoted(&panel.id)?
        )
        .unwrap();
        out.push_str("            uniform token subdivisionScheme = \"none\"\n            uniform token orientation = \"rightHanded\"\n            uniform bool doubleSided = true\n            point3f[] points = [");
        for (j, p) in state.x[panel.vertices.clone()].iter().enumerate() {
            if j > 0 {
                out.push_str(", ");
            }
            write!(out, "({}, {}, {})", p.x as f32, p.y as f32, p.z as f32).unwrap();
        }
        out.push_str("]\n            int[] faceVertexCounts = [");
        for j in 0..panel.triangles.len() {
            if j > 0 {
                out.push_str(", ");
            }
            out.push('3');
        }
        out.push_str("]\n            int[] faceVertexIndices = [");
        let local_triangles: Vec<[usize; 3]> = mesh.triangles[panel.triangles.clone()]
            .iter()
            .map(|tri| tri.vertices.map(|v| v - panel.vertices.start))
            .collect();
        for (j, triangle) in local_triangles.iter().enumerate() {
            if j > 0 {
                out.push_str(", ");
            }
            let [a, b, c] = *triangle;
            write!(out, "{a}, {b}, {c}").unwrap();
        }
        out.push(']');
        push_normals(
            &mut out,
            "            ",
            &state.x[panel.vertices.clone()],
            &local_triangles,
            ShadingMode::Smooth,
        );
        out.push_str("\n            custom double2[] clothcraft:restUV = [");
        for (j, uv) in mesh.uv[panel.vertices.clone()].iter().enumerate() {
            if j > 0 {
                out.push_str(", ");
            }
            write!(out, "({}, {})", uv.x, uv.y).unwrap();
        }
        out.push_str("]\n            texCoord2f[] primvars:st = [");
        for (j, uv) in mesh.uv[panel.vertices.clone()].iter().enumerate() {
            if j > 0 {
                out.push_str(", ");
            }
            write!(out, "({}, {})", uv.x as f32, uv.y as f32).unwrap();
        }
        out.push_str("] (\n                interpolation = \"vertex\"\n            )\n            color3f[] primvars:displayColor = [(0.62, 0.68, 0.78)] (\n                interpolation = \"constant\"\n            )\n        }\n");
    }
    out.push_str("    }\n}\n");
    Ok(out)
}

/// Publish a single-mesh model state as metre-scale, Y-up USDA polygon mesh.
pub fn mesh_stage_usda(
    mesh: &crate::model::Mesh,
    state: &crate::model::State,
) -> Result<String, String> {
    if state.x.len() != mesh.uv.len() || !state.time.is_finite() {
        return Err("USD snapshot requires matching positions and finite time".into());
    }
    if mesh.triangles.is_empty() {
        return Err("USD snapshot requires at least one triangle".into());
    }
    if state
        .x
        .iter()
        .any(|p| p.to_array().into_iter().any(|v| !finite_float(v)))
        || mesh
            .uv
            .iter()
            .any(|p| p.to_array().into_iter().any(|v| !finite_float(v)))
    {
        return Err("USD points and UVs must be finite and representable as f32".into());
    }

    let mut out = String::from(
        "#usda 1.0\n(\n    defaultPrim = \"World\"\n    metersPerUnit = 1\n    upAxis = \"Y\"\n)\n\ndef Xform \"World\"\n{\n    def Scope \"Garment\"\n    {\n        custom int clothcraft:snapshotVersion = 1\n",
    );
    writeln!(
        out,
        "        custom double clothcraft:timeSeconds = {}",
        state.time
    )
    .unwrap();
    out.push_str("        def Mesh \"cloth_mesh\"\n        {\n            custom string clothcraft:panelId = \"cloth\"\n            uniform token subdivisionScheme = \"none\"\n            uniform token orientation = \"rightHanded\"\n            uniform bool doubleSided = true\n            point3f[] points = [");
    for (j, p) in state.x.iter().enumerate() {
        if j > 0 {
            out.push_str(", ");
        }
        write!(out, "({}, {}, {})", p.x as f32, p.y as f32, p.z as f32).unwrap();
    }
    out.push_str("]\n            int[] faceVertexCounts = [");
    for j in 0..mesh.triangles.len() {
        if j > 0 {
            out.push_str(", ");
        }
        out.push('3');
    }
    out.push_str("]\n            int[] faceVertexIndices = [");
    let triangles: Vec<[usize; 3]> = mesh.triangles.iter().map(|tri| tri.vertices).collect();
    for (j, triangle) in triangles.iter().enumerate() {
        if j > 0 {
            out.push_str(", ");
        }
        let [a, b, c] = *triangle;
        write!(out, "{a}, {b}, {c}").unwrap();
    }
    out.push(']');
    push_normals(
        &mut out,
        "            ",
        &state.x,
        &triangles,
        ShadingMode::Smooth,
    );
    out.push_str("\n            custom double2[] clothcraft:restUV = [");
    for (j, uv) in mesh.uv.iter().enumerate() {
        if j > 0 {
            out.push_str(", ");
        }
        write!(out, "({}, {})", uv.x, uv.y).unwrap();
    }
    out.push_str("]\n            texCoord2f[] primvars:st = [");
    for (j, uv) in mesh.uv.iter().enumerate() {
        if j > 0 {
            out.push_str(", ");
        }
        write!(out, "({}, {})", uv.x as f32, uv.y as f32).unwrap();
    }
    out.push_str("] (\n                interpolation = \"vertex\"\n            )\n            color3f[] primvars:displayColor = [(0.62, 0.68, 0.78)] (\n                interpolation = \"constant\"\n            )\n        }\n    }\n}\n");
    Ok(out)
}

/// Update transient display positions on the event thread, preserving USD identity.
#[cfg(feature = "desktop")]
pub fn update_viewport_points(
    scene: &cssimpler::renderer::gpu::usd::UsdScene,
    path: &str,
    positions: &[glam::DVec3],
) -> Result<(), String> {
    use cssimpler::renderer::gpu::openusd::gf;
    if positions
        .iter()
        .any(|p| p.to_array().into_iter().any(|v| !finite_float(v)))
    {
        return Err("USD display positions must be finite f32 values".into());
    }
    let points: Vec<_> = positions
        .iter()
        .map(|p| gf::vec3f(p.x as f32, p.y as f32, p.z as f32))
        .collect();
    scene
        .stage()
        .attribute(format!("{path}.points"))
        .map_err(|e| e.to_string())?
        .set(points)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Transient USD mesh path derived from a semantic panel ID.
#[cfg(feature = "desktop")]
pub fn viewport_panel_path(id: &str) -> String {
    format!("/World/Garment/{}", prim_name(id))
}

/// Author display normals for `path` under `mode`.
///
/// Values and interpolation metadata are rewritten together, so a toggle from
/// `vertex` to `faceVarying` (or back) stays consistent before the next render.
#[cfg(feature = "desktop")]
fn set_viewport_normals(
    scene: &cssimpler::renderer::gpu::usd::UsdScene,
    path: &str,
    points: &[glam::DVec3],
    triangles: &[[usize; 3]],
    mode: ShadingMode,
) -> Result<(), String> {
    use cssimpler::renderer::gpu::openusd::{gf, tf};
    use cssimpler::renderer::gpu::openusd_schemas::geom::AttributeMetadata;
    let values: Vec<_> = display_normals(points, triangles, mode)
        .iter()
        .map(|n| gf::vec3f(n.x as f32, n.y as f32, n.z as f32))
        .collect();
    let attribute = scene
        .stage()
        .attribute(format!("{path}.normals"))
        .map_err(|e| e.to_string())?;
    attribute
        .clone()
        .set_interpolation(tf::Token::new(mode.interpolation()))
        .map_err(|e| e.to_string())?;
    attribute.set(values).map(|_| ()).map_err(|e| e.to_string())
}

/// Recompute one mesh's shading normals after its positions changed.
#[cfg(feature = "desktop")]
pub fn update_viewport_normals(
    scene: &cssimpler::renderer::gpu::usd::UsdScene,
    path: &str,
    points: &[glam::DVec3],
    triangles: &[[usize; 3]],
    mode: ShadingMode,
) -> Result<(), String> {
    set_viewport_normals(scene, path, points, triangles, mode)
}

/// Recompute the static collision display mesh's shading normals.
#[cfg(feature = "desktop")]
pub fn update_viewport_surface_shading(
    scene: &cssimpler::renderer::gpu::usd::UsdScene,
    surface: &crate::avatar::SurfaceMesh,
    mode: ShadingMode,
) -> Result<(), String> {
    set_viewport_normals(
        scene,
        "/Collider",
        &surface.vertices,
        &surface.triangles,
        mode,
    )
}

/// Replace the collision display mesh topology, positions and shading normals.
///
/// Used after a static avatar import. Physics uses the same `SurfaceMesh`; this
/// only updates the transient USD display stage. The caller owns the avatar
/// candidate lifetime.
#[cfg(feature = "desktop")]
pub fn update_viewport_surface(
    scene: &cssimpler::renderer::gpu::usd::UsdScene,
    surface: &crate::avatar::SurfaceMesh,
    mode: ShadingMode,
) -> Result<(), String> {
    if surface
        .vertices
        .iter()
        .any(|p| p.to_array().into_iter().any(|v| !finite_float(v)))
    {
        return Err("USD collider positions must be finite f32 values".into());
    }
    if surface.triangles.is_empty() {
        return Err("USD collider requires at least one triangle".into());
    }
    let counts = vec![3_i32; surface.triangles.len()];
    let indices: Vec<i32> = surface
        .triangles
        .iter()
        .flat_map(|t| t.map(|i| i as i32))
        .collect();
    let stage = scene.stage();
    update_viewport_points(scene, "/Collider", &surface.vertices)?;
    stage
        .attribute("/Collider.faceVertexCounts")
        .map_err(|e| e.to_string())?
        .set(counts)
        .map_err(|e| e.to_string())?;
    stage
        .attribute("/Collider.faceVertexIndices")
        .map_err(|e| e.to_string())?
        .set(indices)
        .map_err(|e| e.to_string())?;
    set_viewport_normals(
        scene,
        "/Collider",
        &surface.vertices,
        &surface.triangles,
        mode,
    )
}

/// Publish one panel's transient positions and shading normals.
#[cfg(feature = "desktop")]
fn set_panel_positions_and_normals(
    scene: &cssimpler::renderer::gpu::usd::UsdScene,
    garment: &CompiledGarment,
    state: &State,
    panel: &crate::garment::template::PanelRange,
    mode: ShadingMode,
) -> Result<(), String> {
    let path = viewport_panel_path(&panel.id);
    let points = &state.x[panel.vertices.clone()];
    update_viewport_points(scene, &path, points)?;
    let local_triangles: Vec<[usize; 3]> = garment.mesh.triangles[panel.triangles.clone()]
        .iter()
        .map(|tri| tri.vertices.map(|v| v - panel.vertices.start))
        .collect();
    set_viewport_normals(scene, &path, points, &local_triangles, mode)
}

/// Refresh every panel's display positions and normals on the event thread.
#[cfg(feature = "desktop")]
pub fn update_panel_viewport_positions(
    scene: &cssimpler::renderer::gpu::usd::UsdScene,
    garment: &CompiledGarment,
    state: &State,
    mode: ShadingMode,
) -> Result<(), String> {
    if state.x.len() != garment.mesh.uv.len() {
        return Err("USD mesh sync requires matching state and UV vertex counts".into());
    }
    if state
        .x
        .iter()
        .any(|p| p.to_array().into_iter().any(|v| !finite_float(v)))
    {
        return Err("USD display positions must be finite f32 values".into());
    }
    for panel in &garment.panels {
        set_panel_positions_and_normals(scene, garment, state, panel, mode)?;
    }
    Ok(())
}

/// Apply a shading mode to every displayed mesh without moving any geometry.
#[cfg(feature = "desktop")]
pub fn apply_viewport_shading(
    scene: &cssimpler::renderer::gpu::usd::UsdScene,
    garment: &CompiledGarment,
    state: &State,
    surface: Option<&crate::avatar::SurfaceMesh>,
    mode: ShadingMode,
) -> Result<(), String> {
    update_panel_viewport_positions(scene, garment, state, mode)?;
    if let Some(surface) = surface {
        update_viewport_surface_shading(scene, surface, mode)?;
    }
    Ok(())
}

#[cfg(feature = "desktop")]
pub fn update_viewport_topology(
    scene: &cssimpler::renderer::gpu::usd::UsdScene,
    mesh: &crate::model::Mesh,
) -> Result<(), String> {
    for (name, values) in [
        ("faceVertexCounts", vec![3_i32; mesh.triangles.len()]),
        (
            "faceVertexIndices",
            mesh.triangles
                .iter()
                .flat_map(|t| t.vertices.map(|i| i as i32))
                .collect(),
        ),
    ] {
        scene
            .stage()
            .attribute(format!("/World/Garment/cloth_mesh.{name}"))
            .map_err(|e| e.to_string())?
            .set(values)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Update per-panel USD mesh topology, vertex positions, normals, and UVs after authoring edits.
#[cfg(feature = "desktop")]
pub fn update_panel_viewport_mesh(
    scene: &cssimpler::renderer::gpu::usd::UsdScene,
    garment: &CompiledGarment,
    state: &State,
    mode: ShadingMode,
) -> Result<(), String> {
    use cssimpler::renderer::gpu::openusd::gf;
    if state.x.len() != garment.mesh.uv.len() {
        return Err("USD mesh sync requires matching state and UV vertex counts".into());
    }
    if state
        .x
        .iter()
        .any(|p| p.to_array().into_iter().any(|v| !finite_float(v)))
        || garment
            .mesh
            .uv
            .iter()
            .any(|p| p.to_array().into_iter().any(|v| !finite_float(v)))
    {
        return Err("USD display positions and UVs must be finite f32 values".into());
    }
    let stage = scene.stage();
    for panel in &garment.panels {
        let path = viewport_panel_path(&panel.id);
        let counts = vec![3_i32; panel.triangles.len()];
        let indices: Vec<i32> = garment.mesh.triangles[panel.triangles.clone()]
            .iter()
            .flat_map(|t| t.vertices.map(|v| (v - panel.vertices.start) as i32))
            .collect();
        let uvs: Vec<_> = garment.mesh.uv[panel.vertices.clone()]
            .iter()
            .map(|uv| gf::vec2f(uv.x as f32, uv.y as f32))
            .collect();
        stage
            .attribute(format!("{path}.faceVertexCounts"))
            .map_err(|e| e.to_string())?
            .set(counts)
            .map_err(|e| e.to_string())?;
        stage
            .attribute(format!("{path}.faceVertexIndices"))
            .map_err(|e| e.to_string())?
            .set(indices)
            .map_err(|e| e.to_string())?;
        stage
            .attribute(format!("{path}.primvars:st"))
            .map_err(|e| e.to_string())?
            .set(uvs)
            .map_err(|e| e.to_string())?;
        set_panel_positions_and_normals(scene, garment, state, panel, mode)?;
    }
    Ok(())
}

/// Add a static collision surface to a display stage without changing physics.
pub fn append_viewport_surface(mut usda: String, surface: &crate::avatar::SurfaceMesh) -> String {
    // Put the display-only mesh alongside World so garment panel identity is unchanged.
    usda.push_str("\ndef Mesh \"Collider\"\n{\n uniform token subdivisionScheme = \"none\"\n uniform bool doubleSided = true\n color3f[] primvars:displayColor = [(0.42, 0.44, 0.48)]\n point3f[] points = [");
    for (i, p) in surface.vertices.iter().enumerate() {
        if i > 0 {
            usda.push(',');
        }
        write!(usda, "({},{},{})", p.x as f32, p.y as f32, p.z as f32).unwrap();
    }
    usda.push_str("]\n int[] faceVertexCounts = [");
    for i in 0..surface.triangles.len() {
        if i > 0 {
            usda.push(',');
        }
        usda.push('3');
    }
    usda.push_str("]\n int[] faceVertexIndices = [");
    for (i, t) in surface.triangles.iter().enumerate() {
        if i > 0 {
            usda.push(',');
        }
        write!(usda, "{},{},{}", t[0], t[1], t[2]).unwrap();
    }
    usda.push(']');
    push_normals(
        &mut usda,
        " ",
        &surface.vertices,
        &surface.triangles,
        ShadingMode::Smooth,
    );
    usda.push_str("\n}\n");
    usda
}

/// Tessellate the analytic reference collider for display only.
pub fn viewport_sphere(sphere: &crate::contact::Sphere) -> crate::avatar::SurfaceMesh {
    use glam::DVec3;
    let mut vertices = vec![sphere.center + DVec3::Y * sphere.radius];
    let mut triangles = Vec::new();
    const N: usize = 32;
    const R: usize = 15;
    for ring in 1..=R {
        let theta = std::f64::consts::PI * ring as f64 / (R + 1) as f64;
        for j in 0..N {
            let phi = std::f64::consts::TAU * j as f64 / N as f64;
            vertices.push(
                sphere.center
                    + sphere.radius
                        * DVec3::new(
                            theta.sin() * phi.cos(),
                            theta.cos(),
                            theta.sin() * phi.sin(),
                        ),
            );
        }
    }
    let bottom = vertices.len();
    vertices.push(sphere.center - DVec3::Y * sphere.radius);
    for j in 0..N {
        let next = (j + 1) % N;
        triangles.push([0, 1 + next, 1 + j]);
        for ring in 0..R - 1 {
            let a = 1 + ring * N + j;
            let b = 1 + ring * N + next;
            triangles.push([a, b, a + N]);
            triangles.push([b, b + N, a + N]);
        }
        triangles.push([1 + (R - 1) * N + j, 1 + (R - 1) * N + next, bottom]);
    }
    crate::avatar::SurfaceMesh {
        vertices,
        triangles,
    }
}

#[cfg(feature = "desktop")]
pub const VIEWPORT_CAMERA_PATH: &str = "/World/ViewportCamera";

#[cfg(feature = "desktop")]
pub fn default_scene_camera(
    positions: &[glam::DVec3],
    surface: Option<&crate::avatar::SurfaceMesh>,
) -> crate::render::Camera {
    let mut min = glam::DVec3::splat(f64::INFINITY);
    let mut max = glam::DVec3::splat(f64::NEG_INFINITY);
    let mut any = false;
    let mut include = |pts: &[glam::DVec3]| {
        for &p in pts {
            if p.is_finite() {
                min = min.min(p);
                max = max.max(p);
                any = true;
            }
        }
    };
    include(positions);
    if let Some(surface) = surface {
        include(&surface.vertices);
    }
    if !any {
        return crate::render::Camera::default();
    }
    let center = ((min + max) * 0.5).as_vec3();
    let extent = (max - min).length().max(0.01) as f32;
    let distance = (extent * 1.6).clamp(0.5, 15.0);
    crate::render::Camera::new(0.5, 0.25, distance, center)
}

#[cfg(feature = "desktop")]
pub fn init_viewport_camera(
    scene: &cssimpler::renderer::gpu::usd::UsdScene,
    camera: &crate::render::Camera,
) -> Result<(), String> {
    use cssimpler::renderer::gpu::openusd::sdf;
    use cssimpler::renderer::gpu::openusd_schemas::geom::{self, CameraSchema};
    let stage = scene.stage();
    let path = sdf::Path::new(VIEWPORT_CAMERA_PATH).map_err(|e| e.to_string())?;
    let cam = geom::Camera::define(stage, &path).map_err(|e| e.to_string())?;
    cam.create_focal_length_attr()
        .map_err(|e| e.to_string())?
        .set(35.0_f32)
        .map_err(|e| e.to_string())?;
    cam.create_vertical_aperture_attr()
        .map_err(|e| e.to_string())?
        .set(24.0_f32)
        .map_err(|e| e.to_string())?;
    cam.create_horizontal_aperture_attr()
        .map_err(|e| e.to_string())?
        .set(36.0_f32)
        .map_err(|e| e.to_string())?;
    update_viewport_camera(scene, camera)
}

#[cfg(feature = "desktop")]
pub fn update_viewport_camera(
    scene: &cssimpler::renderer::gpu::usd::UsdScene,
    camera: &crate::render::Camera,
) -> Result<(), String> {
    use cssimpler::renderer::gpu::openusd::{gf, sdf};
    use cssimpler::renderer::gpu::openusd_schemas::geom::{self, CameraSchema, XformableExt};
    let stage = scene.stage();
    let path = sdf::Path::new(VIEWPORT_CAMERA_PATH).map_err(|e| e.to_string())?;
    let cam = geom::Camera::get(stage, &path)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("USD camera not found: {VIEWPORT_CAMERA_PATH}"))?;
    let world =
        glam::camera::rh::view::look_at_mat4(camera.eye(), camera.target, camera.up()).inverse();
    let dist = camera.distance.max(0.01);
    cam.create_clipping_range_attr()
        .map_err(|e| e.to_string())?
        .set(gf::vec2f((dist * 0.001).max(0.0001), dist * 1000.0))
        .map_err(|e| e.to_string())?;
    cam.set_transform(gf::Matrix4d(world.to_cols_array().map(f64::from)))
        .map_err(|e| e.to_string())?;
    Ok(())
}
