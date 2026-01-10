use fathom::app::icons;

#[test]
fn svg_placeholders_present() {
    assert!(icons::svg_source("storage").is_some());
    assert!(icons::svg_source("pvc").is_some());
    assert!(icons::svg_source("disk").is_some());
}

#[test]
fn svg_rasterization_roundtrips() {
    let svg = icons::svg_source("storage").unwrap();
    let img = icons::rasterize_svg_to_color_image(svg, 32).expect("Rasterize should succeed");
    assert_eq!(img.size, [32, 32]);
}
