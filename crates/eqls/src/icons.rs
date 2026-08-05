use image::{ImageFormat, RgbaImage};
use std::io::Cursor;

/// `dragitem{n}.dds`: a 6x6 grid of 40x40 cells, origin top-left, no padding.
const GRID: u32 = 6;
const CELL: u32 = 40;
const PER_SHEET: i32 = (GRID * GRID) as i32;

const PF_FOURCC: u32 = 0x4;
const PF_RGB: u32 = 0x40;

const FIRST_ICON: i32 = 500;
const SHEETS: u32 = 379;
const LAST_ICON: i32 = FIRST_ICON + SHEETS as i32 * PER_SHEET - 1;

#[derive(Debug, thiserror::Error)]
pub enum IconError {
    #[error("sheet must be between 1 and {SHEETS}, got {0}")]
    BadSheet(u32),
    #[error("sheet must be at least {}x{} pixels, got {width}x{height}", GRID * CELL, GRID * CELL)]
    BadSize { width: u32, height: u32 },
    #[error("sheet is not a readable dds: {0}")]
    Decode(#[from] image::ImageError),
}

/// Cells run top-to-bottom then left-to-right: id = base + col * 6 + row.
pub fn locate(icon: i32) -> Option<(u32, u32, u32)> {
    if !(FIRST_ICON..=LAST_ICON).contains(&icon) {
        return None;
    }
    let n = icon - FIRST_ICON;
    let i = (n % PER_SHEET) as u32;
    Some(((n / PER_SHEET) as u32 + 1, i / GRID, i % GRID))
}

pub fn icon_at(sheet: u32, col: u32, row: u32) -> i32 {
    FIRST_ICON + (sheet as i32 - 1) * PER_SHEET + (col * GRID + row) as i32
}

/// `image` handles only the BC/DXT variants; a few sheets ship as plain 32-bit
/// pixels and have to be unpacked here.
fn decode_sheet(dds: &[u8]) -> Result<RgbaImage, IconError> {
    match unpack_rgba(dds) {
        Some(image) => Ok(image),
        None => Ok(image::load_from_memory_with_format(dds, ImageFormat::Dds)?.to_rgba8()),
    }
}

fn unpack_rgba(dds: &[u8]) -> Option<RgbaImage> {
    let word = |at: usize| -> Option<u32> {
        Some(u32::from_le_bytes(dds.get(at..at + 4)?.try_into().ok()?))
    };
    if !dds.starts_with(b"DDS ") {
        return None;
    }
    let (height, width) = (word(12)?, word(16)?);
    let pf_flags = word(80)?;
    if pf_flags & PF_FOURCC != 0 || pf_flags & PF_RGB == 0 || word(88)? != 32 {
        return None;
    }
    let masks = [word(92)?, word(96)?, word(100)?, word(104)?];

    let level = dds.get(128..128 + width as usize * height as usize * 4)?;
    let mut image = RgbaImage::new(width, height);
    for (pixel, chunk) in image.pixels_mut().zip(level.chunks_exact(4)) {
        let packed = u32::from_le_bytes(chunk.try_into().ok()?);
        for (channel, mask) in pixel.0.iter_mut().zip(masks) {
            *channel = match mask {
                0 => 255,
                mask => ((packed & mask) >> mask.trailing_zeros()) as u8,
            };
        }
    }
    Some(image)
}

/// Fully transparent cells are dropped so their ids 404 rather than serving an
/// invisible PNG.
pub fn split_sheet(sheet: u32, dds: &[u8]) -> Result<Vec<(i32, Vec<u8>)>, IconError> {
    if !(1..=SHEETS).contains(&sheet) {
        return Err(IconError::BadSheet(sheet));
    }
    let image = decode_sheet(dds)?;
    if image.width() < GRID * CELL || image.height() < GRID * CELL {
        return Err(IconError::BadSize {
            width: image.width(),
            height: image.height(),
        });
    }

    let mut icons = Vec::with_capacity(PER_SHEET as usize);
    for col in 0..GRID {
        for row in 0..GRID {
            let cell = image::imageops::crop_imm(&image, col * CELL, row * CELL, CELL, CELL);
            let cell: RgbaImage = cell.to_image();
            if cell.pixels().all(|pixel| pixel.0[3] == 0) {
                continue;
            }
            let mut png = Vec::new();
            cell.write_to(&mut Cursor::new(&mut png), ImageFormat::Png)?;
            icons.push((icon_at(sheet, col, row), png));
        }
    }
    Ok(icons)
}

/// A 256x256 DXT5 sheet whose every 4x4 block is a flat colour keyed to the cell
/// it falls in, so a crop can be read back as its (col, row).
#[cfg(test)]
pub(crate) fn dxt5_sheet(blank: &[(u32, u32)]) -> Vec<u8> {
    let mut dds = dds_header(0x0008_1007, 65536, 0, PF_FOURCC, b"DXT5", 0, [0; 4]);
    for block_y in 0..64u32 {
        for block_x in 0..64u32 {
            let (col, row) = (block_x * 4 / CELL, block_y * 4 / CELL);
            let alpha = if is_blank(col, row, blank) { 0u8 } else { 255 };
            dds.extend_from_slice(&[alpha, alpha, 0, 0, 0, 0, 0, 0]);
            let colour = ((col as u16 + 1) << 11) | ((row as u16 + 1) << 5);
            dds.extend_from_slice(&colour.to_le_bytes());
            dds.extend_from_slice(&colour.to_le_bytes());
            dds.extend_from_slice(&[0, 0, 0, 0]);
        }
    }
    dds
}

/// The same grid as an uncompressed 32-bit BGRA sheet, the variant `image`
/// refuses and `unpack_rgba` has to cover.
#[cfg(test)]
pub(crate) fn bgra_sheet(blank: &[(u32, u32)]) -> Vec<u8> {
    let masks = [0x00FF_0000, 0x0000_FF00, 0x0000_00FF, 0xFF00_0000];
    let mut dds = dds_header(0x0000_100F, 1024, 1, PF_RGB | 0x1, &[0; 4], 32, masks);
    for y in 0..256u32 {
        for x in 0..256u32 {
            let (col, row) = (x / CELL, y / CELL);
            if is_blank(col, row, blank) {
                dds.extend_from_slice(&[0, 0, 0, 0]);
            } else {
                dds.extend_from_slice(&[(row as u8 + 1) * 20, (col as u8 + 1) * 20, 200, 255]);
            }
        }
    }
    dds
}

#[cfg(test)]
fn is_blank(col: u32, row: u32, blank: &[(u32, u32)]) -> bool {
    col >= GRID || row >= GRID || blank.contains(&(col, row))
}

#[cfg(test)]
fn dds_header(
    flags: u32,
    linear_size: u32,
    mips: u32,
    pf_flags: u32,
    fourcc: &[u8],
    bits: u32,
    masks: [u32; 4],
) -> Vec<u8> {
    let mut dds = Vec::with_capacity(128 + 65536);
    dds.extend_from_slice(b"DDS ");
    for word in [124, flags, 256, 256, linear_size, 0, mips] {
        dds.extend_from_slice(&u32::to_le_bytes(word));
    }
    dds.extend_from_slice(&[0u8; 44]);
    dds.extend_from_slice(&32u32.to_le_bytes());
    dds.extend_from_slice(&pf_flags.to_le_bytes());
    dds.extend_from_slice(fourcc);
    dds.extend_from_slice(&bits.to_le_bytes());
    for mask in masks {
        dds.extend_from_slice(&mask.to_le_bytes());
    }
    dds.extend_from_slice(&0x1000u32.to_le_bytes());
    dds.extend_from_slice(&[0u8; 16]);
    assert_eq!(dds.len(), 128);
    dds
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verified against PQDI's per-icon reference art.
    const KNOWN: &[(i32, u32, u32, u32)] = &[
        (505, 1, 0, 5),
        (516, 1, 2, 4),
        (524, 1, 4, 0),
        (531, 1, 5, 1),
        (540, 2, 0, 4),
        (550, 2, 2, 2),
        (564, 2, 4, 4),
        (591, 3, 3, 1),
        (624, 4, 2, 4),
        (626, 4, 3, 0),
        (634, 4, 4, 2),
        (661, 5, 2, 5),
        (756, 8, 0, 4),
        (769, 8, 2, 5),
        (770, 8, 3, 0),
        (880, 11, 3, 2),
    ];

    #[test]
    fn locate_matches_the_reference_icons() {
        for &(icon, sheet, col, row) in KNOWN {
            assert_eq!(locate(icon), Some((sheet, col, row)), "icon {icon}");
        }
    }

    #[test]
    fn locate_and_icon_at_round_trip_over_every_cell() {
        for icon in FIRST_ICON..=LAST_ICON {
            let (sheet, col, row) = locate(icon).unwrap();
            assert_eq!(icon_at(sheet, col, row), icon);
            assert!((1..=SHEETS).contains(&sheet));
            assert!(col < GRID && row < GRID);
        }
    }

    #[test]
    fn ids_outside_the_bank_have_no_cell() {
        assert_eq!(locate(FIRST_ICON - 1), None);
        assert_eq!(locate(LAST_ICON + 1), None);
        assert_eq!(locate(0), None);
        assert_eq!(locate(-1), None);
        assert_eq!(LAST_ICON, 14143);
        assert!(locate(FIRST_ICON).is_some() && locate(LAST_ICON).is_some());
    }

    fn expand5(value: u8) -> u8 {
        (value as u32 * 255 / 31) as u8
    }

    fn expand6(value: u8) -> u8 {
        (value as u32 * 255 / 63) as u8
    }

    #[test]
    fn split_sheet_crops_every_cell_at_the_right_coordinates() {
        let dds = dxt5_sheet(&[]);
        let cells = split_sheet(4, &dds).unwrap();
        assert_eq!(cells.len(), 36);

        let ids: Vec<i32> = cells.iter().map(|(icon, _)| *icon).collect();
        assert_eq!(ids[0], 608);
        assert_eq!(ids[35], 643);
        assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));

        for (icon, png) in &cells {
            let (sheet, col, row) = locate(*icon).unwrap();
            assert_eq!(sheet, 4);
            let cell = image::load_from_memory_with_format(png, ImageFormat::Png)
                .unwrap()
                .to_rgba8();
            assert_eq!(cell.dimensions(), (CELL, CELL));
            let pixel = cell.get_pixel(20, 20).0;
            assert_eq!(pixel[0], expand5(col as u8 + 1), "icon {icon} column");
            assert_eq!(pixel[1], expand6(row as u8 + 1), "icon {icon} row");
            assert_eq!(pixel[3], 255);
        }

        let (_, col, row) = locate(624).unwrap();
        assert_eq!(cells[(col * GRID + row) as usize].0, 624);
    }

    #[test]
    fn uncompressed_sheets_decode_to_the_same_grid() {
        let cells = split_sheet(1, &bgra_sheet(&[(2, 3)])).unwrap();
        assert_eq!(cells.len(), 35);
        assert!(!cells.iter().any(|(icon, _)| *icon == icon_at(1, 2, 3)));

        for (icon, png) in &cells {
            let (_, col, row) = locate(*icon).unwrap();
            let cell = image::load_from_memory_with_format(png, ImageFormat::Png)
                .unwrap()
                .to_rgba8();
            assert_eq!(cell.dimensions(), (CELL, CELL));
            assert_eq!(
                cell.get_pixel(20, 20).0,
                [200, (col as u8 + 1) * 20, (row as u8 + 1) * 20, 255],
                "icon {icon}"
            );
        }
    }

    #[test]
    fn split_sheet_drops_blank_cells_and_rejects_bad_input() {
        let dds = dxt5_sheet(&[(0, 0), (5, 5)]);
        let cells = split_sheet(1, &dds).unwrap();
        assert_eq!(cells.len(), 34);
        assert!(!cells.iter().any(|(icon, _)| *icon == 500 || *icon == 535));

        assert!(matches!(split_sheet(0, &dds), Err(IconError::BadSheet(0))));
        assert!(matches!(
            split_sheet(SHEETS + 1, &dds),
            Err(IconError::BadSheet(380))
        ));
        assert!(matches!(
            split_sheet(1, b"not a dds"),
            Err(IconError::Decode(_))
        ));
    }
}
