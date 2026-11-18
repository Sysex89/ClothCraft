//! Property inspector values read from the document.

use super::super::ui::inspector::properties::fabric_mechanics_rows;
use clothcraft::fitting::FittingProject;
use clothcraft::garment::presets::GarmentKind;

#[test]
fn fabric_mechanics_rows_reflect_the_selected_panels_fabric() {
    let mut template = FittingProject::basic(GarmentKind::Tshirt, 1, "unused.obj".into()).template;
    assert!(template.panels.len() >= 2);
    let mut denim = template.fabrics[0].clone();
    denim.id = "test-denim".into();
    denim.name = "Test denim".into();
    denim.properties.density = 0.42;
    denim.properties.warp = 900.0;
    denim.properties.bend_warp = 3.5e-5;
    template.fabrics.push(denim);
    template.panels[1].fabric = "test-denim".into();

    let first = fabric_mechanics_rows(&template, 0);
    let second = fabric_mechanics_rows(&template, 1);
    let value = |rows: &[(&str, String)], label: &str| {
        rows.iter()
            .find(|(l, _)| *l == label)
            .map(|(_, v)| v.clone())
    };
    assert_eq!(
        value(&second, "Fabric").as_deref(),
        Some("Test denim (test-denim)")
    );
    assert_eq!(value(&second, "Areal Density").as_deref(), Some("420 g/m²"));
    assert_eq!(value(&second, "Warp Membrane").as_deref(), Some("900 N/m"));
    assert_eq!(
        value(&second, "Bending Warp").as_deref(),
        Some("3.50e-5 N·m")
    );
    let base = &template.fabrics[0].properties;
    assert_eq!(
        value(&first, "Areal Density"),
        Some(format!("{:.0} g/m²", base.density * 1e3))
    );
    assert_ne!(first, second, "different fabrics must render differently");

    assert_eq!(
        fabric_mechanics_rows(&template, template.panels.len()),
        vec![("Fabric", "No panel selected".to_string())]
    );
    template.panels[0].fabric = "missing".into();
    assert_eq!(
        fabric_mechanics_rows(&template, 0),
        vec![("Fabric", "Unknown fabric id 'missing'".to_string())]
    );
}
