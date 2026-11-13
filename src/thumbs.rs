//! Model thumbnails: a `SIZE` x `SIZE` PNG of the iso view of each model in
//! `<config>/thumbs/`. CoCAD writes it when a model opens (after the first
//! build) and when the model closes. The start page shows it.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Width and height of a thumbnail in pixels.
pub const SIZE: u32 = 512;

/// The offscreen render is this many times larger, for smooth edges.
pub const SUPERSAMPLE: u32 = 2;

/// FNV-1a 64: a hash that stays the same between runs and versions.
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
    })
}

fn file_in(dir: &Path, scad: &Path) -> PathBuf {
    let key = scad.canonicalize().unwrap_or_else(|_| scad.to_path_buf());
    let name = format!("{:016x}.png", fnv(key.to_string_lossy().as_bytes()));
    dir.join(name)
}

/// The thumbnail file of a model.
pub fn file_for(scad: &Path) -> Option<PathBuf> {
    Some(file_in(
        &crate::openscad::config_dir()?.join("thumbs"),
        scad,
    ))
}

/// Makes an image `factor` times smaller. Each output pixel is the mean of
/// a block of input pixels, with alpha weights: the transparent background
/// does not make the edges dark.
pub fn shrink(rgba: &[u8], size: u32, factor: u32) -> Vec<u8> {
    let out = size / factor;
    let mut px = Vec::with_capacity((out * out * 4) as usize);
    let n = factor * factor;
    for y in 0..out {
        for x in 0..out {
            let mut sum = [0u32; 4];
            for dy in 0..factor {
                for dx in 0..factor {
                    let i = (((y * factor + dy) * size + x * factor + dx) * 4) as usize;
                    let a = u32::from(rgba[i + 3]);
                    for c in 0..3 {
                        sum[c] += u32::from(rgba[i + c]) * a;
                    }
                    sum[3] += a;
                }
            }
            for c in 0..3 {
                px.push(sum[c].checked_div(sum[3]).unwrap_or(0) as u8);
            }
            px.push((sum[3] / n) as u8);
        }
    }
    px
}

/// Writes an RGBA image as the thumbnail of `scad`. A temporary file and a
/// rename make sure that a reader never sees half a file.
pub fn save(scad: &Path, rgba: &[u8], size: u32) -> Result<()> {
    let file = file_for(scad).context("no config folder")?;
    save_to(&file, rgba, size)
}

fn save_to(file: &Path, rgba: &[u8], size: u32) -> Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = file.with_extension("png.tmp");
    {
        let w = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
        let mut enc = png::Encoder::new(w, size, size);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header()?.write_image_data(rgba)?;
    }
    std::fs::rename(&tmp, file)?;
    Ok(())
}

/// Reads a thumbnail file: width, height and RGBA pixels.
pub fn load(file: &Path) -> Option<(usize, usize, Vec<u8>)> {
    decode(&std::fs::read(file).ok()?)
}

/// Decodes an RGBA PNG: width, height and pixels.
pub fn decode(bytes: &[u8]) -> Option<(usize, usize, Vec<u8>)> {
    let mut dec = png::Decoder::new(bytes);
    dec.set_transformations(png::Transformations::EXPAND);
    let mut reader = dec.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    buf.truncate(info.buffer_size());
    Some((info.width as usize, info.height as usize, buf))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shrink_keeps_edges_bright() {
        // A 2 x 2 block: one white opaque pixel, three transparent pixels.
        let mut img = vec![0u8; 16];
        img[..4].copy_from_slice(&[255, 255, 255, 255]);
        let out = shrink(&img, 2, 2);
        assert_eq!(out, [255, 255, 255, 63]);
        // All transparent: black and transparent, no division by 0.
        assert_eq!(shrink(&[0; 16], 2, 2), [0, 0, 0, 0]);
    }

    #[test]
    fn file_name_depends_on_the_path() {
        let dir = Path::new("/thumbs");
        let a = file_in(dir, Path::new("/no/such/a.scad"));
        let b = file_in(dir, Path::new("/no/such/b.scad"));
        assert_ne!(a, b);
        assert_eq!(a, file_in(dir, Path::new("/no/such/a.scad")));
        assert_eq!(a.extension().unwrap(), "png");
    }

    #[test]
    fn save_and_load() {
        let dir = std::env::temp_dir().join(format!("cocad-thumbs-{}", std::process::id()));
        let file = dir.join("t.png");
        let img: Vec<u8> = (0..4 * 4 * 4).map(|i| i as u8).collect();
        save_to(&file, &img, 4).unwrap();
        let (w, h, px) = load(&file).unwrap();
        assert_eq!((w, h), (4, 4));
        assert_eq!(px, img);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
