fn main() {
    // Vendored Hubble/Cilium proto subset (proto/hubble/) -> generated gRPC client.
    // protobuf-src builds libprotobuf from source, so no system `protoc` is required
    // on any Tier-1 target (Ubuntu 26 / Windows 11 / macOS 26).
    unsafe {
        std::env::set_var("PROTOC", protobuf_src::protoc());
    }
    tonic_prost_build::configure()
        .build_server(false)
        .compile_protos(
            &[
                "proto/hubble/observer/observer.proto",
                "proto/hubble/flow/flow.proto",
                "proto/hubble/relay/relay.proto",
            ],
            &["proto/hubble"],
        )
        .expect("failed to compile vendored Hubble proto files");
    println!("cargo:rerun-if-changed=proto/hubble/");

    // Only compile resources on Windows
    #[cfg(windows)]
    {
        // Convert PNG to ICO if needed
        let icon_path = Path::new("assets/icon.png");
        let ico_path = Path::new("assets/icon.ico");

        if icon_path.exists()
            && !ico_path.exists()
            && let Err(e) = convert_png_to_ico(icon_path, ico_path)
        {
            println!("cargo:warning=Failed to convert PNG to ICO: {}", e);
        }

        // Embed the icon resource
        if ico_path.exists() {
            let _ = embed_resource::compile("fathom.rc", embed_resource::NONE);
        } else {
            println!("cargo:warning=Icon file not found at assets/icon.ico");
        }
    }

    // Rerun if assets change
    println!("cargo:rerun-if-changed=assets/");
    println!("cargo:rerun-if-changed=fathom.rc");
}

#[cfg(windows)]
fn convert_png_to_ico(png_path: &Path, ico_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    use image::ImageFormat;
    use image::imageops::FilterType;
    use std::fs::File;
    use std::io::Write;

    // Load the PNG image
    let img = image::open(png_path)?;

    // Create multiple sizes for the ICO file (16, 32, 48, 256)
    let sizes = [256, 48, 32, 16];
    let mut ico_data = Vec::new();

    // ICO header
    ico_data.write_all(&[0, 0])?; // Reserved
    ico_data.write_all(&[1, 0])?; // Type: ICO
    ico_data.write_all(&(sizes.len() as u16).to_le_bytes())?; // Number of images

    let mut images = Vec::new();
    let mut offset = 6 + (16 * sizes.len()); // Header + directory entries

    // Create resized images and directory entries
    for &size in &sizes {
        let resized = img.resize_exact(size, size, FilterType::Lanczos3);
        let mut png_bytes = Vec::new();
        resized.write_to(&mut std::io::Cursor::new(&mut png_bytes), ImageFormat::Png)?;

        // ICO directory entry
        ico_data.push(if size == 256 { 0 } else { size as u8 }); // Width
        ico_data.push(if size == 256 { 0 } else { size as u8 }); // Height
        ico_data.push(0); // Color palette
        ico_data.push(0); // Reserved
        ico_data.write_all(&1u16.to_le_bytes())?; // Color planes
        ico_data.write_all(&32u16.to_le_bytes())?; // Bits per pixel
        ico_data.write_all(&(png_bytes.len() as u32).to_le_bytes())?; // Size of image data
        ico_data.write_all(&(offset as u32).to_le_bytes())?; // Offset

        offset += png_bytes.len();
        images.push(png_bytes);
    }

    // Append all image data
    for img_data in images {
        ico_data.write_all(&img_data)?;
    }

    // Write ICO file
    let mut file = File::create(ico_path)?;
    file.write_all(&ico_data)?;

    Ok(())
}
