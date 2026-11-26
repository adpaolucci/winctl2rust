use std::sync::Arc;

use guap_table::{
    CellValue, ColumnDef, ColumnType, Row, TableData, TableError, TableProvider, TableQuery, TableSchema, apply_query,
};
use serde::{Deserialize, Serialize};
use winctl2rust_core::{WinctlError, WinctlResult};

use crate::backend::{DisplayBackend, RawEdidEntry};

pub const EDID_HEADER_MAGIC: [u8; 8] = [0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EdidVideoInput {
    pub is_digital: bool,
    pub color_bit_depth: Option<u8>,
    pub digital_interface: Option<String>,
    pub dfp_1x_compatible: Option<bool>,
    pub analog_signal_level: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EdidChromaticity {
    pub red_x: f32,
    pub red_y: f32,
    pub green_x: f32,
    pub green_y: f32,
    pub blue_x: f32,
    pub blue_y: f32,
    pub white_x: f32,
    pub white_y: f32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EdidStandardTiming {
    pub width: u32,
    pub height: u32,
    pub refresh_hz: u32,
    pub aspect_ratio: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EdidDetailedTiming {
    pub pixel_clock_khz: u32,
    pub h_active: u16,
    pub h_blank: u16,
    pub v_active: u16,
    pub v_blank: u16,
    pub h_sync_offset: u16,
    pub h_sync_pulse: u16,
    pub v_sync_offset: u16,
    pub v_sync_pulse: u16,
    pub h_size_mm: u16,
    pub v_size_mm: u16,
    pub interlaced: bool,
    pub refresh_hz: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EdidRangeLimits {
    pub min_v_hz: u8,
    pub max_v_hz: u8,
    pub min_h_khz: u8,
    pub max_h_khz: u8,
    pub max_pixel_clock_mhz: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CeaAudioDescriptor {
    pub format_code: u8,
    pub format_name: String,
    pub max_channels: u8,
    pub sample_rates_mask: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CeaVideoDescriptor {
    pub vic: u8,
    pub native: bool,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CeaVendorSpecific {
    pub ieee_oui: String,
    pub vendor_name: String,
    pub max_tmds_mhz: Option<u32>,
    pub raw_payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cea861Extension {
    pub revision: u8,
    pub checksum_valid: bool,
    pub underscan: bool,
    pub basic_audio: bool,
    pub ycbcr_444: bool,
    pub ycbcr_422: bool,
    pub native_dtd_count: u8,
    pub audio_descriptors: Vec<CeaAudioDescriptor>,
    pub video_descriptors: Vec<CeaVideoDescriptor>,
    pub vendor_specific: Vec<CeaVendorSpecific>,
    pub speaker_allocation_mask: Option<u32>,
    pub extended_tags: Vec<u8>,
    pub detailed_timings: Vec<EdidDetailedTiming>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParsedEdid {
    pub manufacturer_id: String,
    pub product_code: u16,
    pub product_code_hex: String,
    pub serial_number_u32: u32,
    pub manufacture_week: Option<u8>,
    pub manufacture_year: u16,
    pub is_model_year: bool,
    pub edid_version: String,
    pub video_input: EdidVideoInput,
    pub width_cm: Option<u8>,
    pub height_cm: Option<u8>,
    pub diagonal_inches: Option<f32>,
    pub aspect_ratio_hint: Option<f32>,
    pub gamma: Option<f32>,
    pub standby: bool,
    pub suspend: bool,
    pub active_off: bool,
    pub srgb_default: bool,
    pub preferred_timing_mode: bool,
    pub continuous_freq: bool,
    pub chromaticity: EdidChromaticity,
    pub established_timings: Vec<String>,
    pub standard_timings: Vec<EdidStandardTiming>,
    pub detailed_timings: Vec<EdidDetailedTiming>,
    pub monitor_name: Option<String>,
    pub ascii_serial: Option<String>,
    pub unspecified_texts: Vec<String>,
    pub range_limits: Option<EdidRangeLimits>,
    pub extension_count: u8,
    pub base_checksum_valid: bool,
    pub all_checksums_valid: bool,
    pub cea_extensions: Vec<Cea861Extension>,
    pub raw_len: usize,
}

impl ParsedEdid {
    pub fn max_resolution(&self) -> (u32, u32, f64) {
        let mut best: Option<(u32, u32, f64)> = None;
        for dtd in self
            .detailed_timings
            .iter()
            .chain(self.cea_extensions.iter().flat_map(|c| c.detailed_timings.iter()))
        {
            let w = u32::from(dtd.h_active);
            let h = u32::from(dtd.v_active);
            let pixels = u64::from(w) * u64::from(h);
            let replace = match best {
                None => true,
                Some((bw, bh, br)) => {
                    let bp = u64::from(bw) * u64::from(bh);
                    pixels > bp || (pixels == bp && dtd.refresh_hz > br)
                }
            };
            if replace {
                best = Some((w, h, dtd.refresh_hz));
            }
        }
        for st in &self.standard_timings {
            let pixels = u64::from(st.width) * u64::from(st.height);
            let replace = match best {
                None => true,
                Some((bw, bh, _)) => pixels > u64::from(bw) * u64::from(bh),
            };
            if replace {
                best = Some((st.width, st.height, f64::from(st.refresh_hz)));
            }
        }
        best.unwrap_or((0, 0, 0.0))
    }

    pub fn summary_timings(&self) -> String {
        let mut labels = Vec::new();
        for dtd in &self.detailed_timings {
            labels.push(format!("{}x{}@{:.0}Hz", dtd.h_active, dtd.v_active, dtd.refresh_hz));
        }
        for cea in &self.cea_extensions {
            for dtd in &cea.detailed_timings {
                labels.push(format!("{}x{}@{:.0}Hz", dtd.h_active, dtd.v_active, dtd.refresh_hz));
            }
            for svd in &cea.video_descriptors {
                labels.push(svd.label.clone());
            }
        }
        for st in &self.standard_timings {
            labels.push(format!("{}x{}@{}Hz", st.width, st.height, st.refresh_hz));
        }
        labels.extend(self.established_timings.iter().cloned());
        labels.dedup();
        labels.truncate(8);
        labels.join(", ")
    }
}

pub fn verify_edid_block_checksum(block: &[u8]) -> bool {
    block.len() == 128 && block.iter().fold(0u8, |acc, &b| acc.wrapping_add(b)) == 0
}

pub fn compute_edid_checksum_byte(first_127: &[u8]) -> u8 {
    let sum = first_127[..127].iter().fold(0u8, |acc, &b| acc.wrapping_add(b));
    0u8.wrapping_sub(sum)
}

pub fn decode_pnp_id(b0: u8, b1: u8) -> String {
    let raw = u16::from_be_bytes([b0, b1]);
    let c1 = ((raw >> 10) & 0x1F) as u8;
    let c2 = ((raw >> 5) & 0x1F) as u8;
    let c3 = (raw & 0x1F) as u8;
    let map_char = |c: u8| -> char { if (1..=26).contains(&c) { (b'@' + c) as char } else { '?' } };
    format!("{}{}{}", map_char(c1), map_char(c2), map_char(c3))
}

fn decode_dtd(d: &[u8]) -> Option<EdidDetailedTiming> {
    if d.len() < 18 {
        return None;
    }
    let pix_raw = u16::from_le_bytes([d[0], d[1]]);
    if pix_raw == 0 {
        return None;
    }
    let pixel_clock_khz = u32::from(pix_raw) * 10;
    let h_active = u16::from(d[2]) | (u16::from(d[4] >> 4) << 8);
    let h_blank = u16::from(d[3]) | (u16::from(d[4] & 0x0F) << 8);
    let v_active = u16::from(d[5]) | (u16::from(d[7] >> 4) << 8);
    let v_blank = u16::from(d[6]) | (u16::from(d[7] & 0x0F) << 8);
    let h_sync_offset = u16::from(d[8]) | (u16::from((d[11] >> 6) & 0x03) << 8);
    let h_sync_pulse = u16::from(d[9]) | (u16::from((d[11] >> 4) & 0x03) << 8);
    let v_sync_offset = u16::from(d[10] >> 4) | (u16::from((d[11] >> 2) & 0x03) << 4);
    let v_sync_pulse = u16::from(d[10] & 0x0F) | (u16::from(d[11] & 0x03) << 4);
    let h_size_mm = u16::from(d[12]) | (u16::from(d[14] >> 4) << 8);
    let v_size_mm = u16::from(d[13]) | (u16::from(d[14] & 0x0F) << 8);
    let interlaced = (d[17] & 0x80) != 0;

    let h_total = u32::from(h_active) + u32::from(h_blank);
    let v_total = u32::from(v_active) + u32::from(v_blank);
    let refresh_hz = if h_total > 0 && v_total > 0 {
        (f64::from(pixel_clock_khz) * 1000.0) / (f64::from(h_total) * f64::from(v_total))
    } else {
        0.0
    };

    Some(EdidDetailedTiming {
        pixel_clock_khz,
        h_active,
        h_blank,
        v_active,
        v_blank,
        h_sync_offset,
        h_sync_pulse,
        v_sync_offset,
        v_sync_pulse,
        h_size_mm,
        v_size_mm,
        interlaced,
        refresh_hz: (refresh_hz * 100.0).round() / 100.0,
    })
}

fn decode_edid_ascii(raw: &[u8]) -> String {
    let mut s = String::new();
    for &b in raw {
        if b == 0x0A || b == 0x00 {
            break;
        }
        if (0x20..=0x7E).contains(&b) {
            s.push(b as char);
        }
    }
    s.trim().to_string()
}

fn decode_established_timings(b: &[u8]) -> Vec<String> {
    const BYTE_35: [&str; 8] = [
        "800x600@60Hz",
        "800x600@56Hz",
        "640x480@75Hz",
        "640x480@72Hz",
        "640x480@67Hz",
        "640x480@60Hz",
        "720x400@88Hz",
        "720x400@70Hz",
    ];
    const BYTE_36: [&str; 8] = [
        "1280x1024@75Hz",
        "1024x768@75Hz",
        "1024x768@70Hz",
        "1024x768@60Hz",
        "1024x768@87i",
        "832x624@75Hz",
        "800x600@75Hz",
        "800x600@72Hz",
    ];
    let mut out = Vec::new();
    for (bit, label) in BYTE_35.iter().enumerate() {
        if (b[35] & (1 << bit)) != 0 {
            out.push((*label).to_string());
        }
    }
    for (bit, label) in BYTE_36.iter().enumerate() {
        if (b[36] & (1 << bit)) != 0 {
            out.push((*label).to_string());
        }
    }
    if (b[37] & 0x80) != 0 {
        out.push("1152x870@75Hz".to_string());
    }
    out
}

fn decode_standard_timings(b: &[u8], version: u8, revision: u8) -> Vec<EdidStandardTiming> {
    let mut out = Vec::new();
    let is_13_plus = version > 1 || (version == 1 && revision >= 3);
    for pair in b[38..54].as_chunks::<2>().0 {
        let (b0, b1) = (pair[0], pair[1]);
        if (b0 == 0x01 && b1 == 0x01) || (b0 == 0x00 && b1 == 0x00) {
            continue;
        }
        let width = (u32::from(b0) + 31) * 8;
        let aspect_code = (b1 >> 6) & 0x03;
        let (height, aspect_str) = match aspect_code {
            0 => {
                if is_13_plus {
                    (width * 10 / 16, "16:10")
                } else {
                    (width, "1:1")
                }
            }
            1 => (width * 3 / 4, "4:3"),
            2 => (width * 4 / 5, "5:4"),
            _ => (width * 9 / 16, "16:9"),
        };
        let refresh_hz = u32::from(b1 & 0x3F) + 60;
        out.push(EdidStandardTiming {
            width,
            height,
            refresh_hz,
            aspect_ratio: aspect_str.to_string(),
        });
    }
    out
}

fn cea_vic_label(vic: u8) -> String {
    match vic {
        1 => "640x480p60".to_string(),
        2 | 3 => "720x480p60".to_string(),
        4 => "1280x720p60".to_string(),
        5 => "1920x1080i60".to_string(),
        16 => "1920x1080p60".to_string(),
        17 | 18 => "720x576p50".to_string(),
        19 => "1280x720p50".to_string(),
        31 => "1920x1080p50".to_string(),
        32 => "1920x1080p24".to_string(),
        63 => "1920x1080p120".to_string(),
        93 => "3840x2160p24".to_string(),
        95 => "3840x2160p30".to_string(),
        97 => "3840x2160p60".to_string(),
        101 => "4096x2160p60".to_string(),
        other => format!("VIC {other}"),
    }
}

fn cea_audio_format_name(code: u8) -> &'static str {
    match code {
        1 => "LPCM",
        2 => "AC-3",
        3 => "MPEG-1",
        4 => "MP3",
        5 => "MPEG-2",
        6 => "AAC-LC",
        7 => "DTS",
        8 => "ATRAC",
        9 => "DSD",
        10 => "E-AC-3",
        11 => "DTS-HD",
        12 => "Dolby TrueHD",
        _ => "Other",
    }
}

fn parse_cea861_extension(ext: &[u8]) -> Cea861Extension {
    let checksum_valid = verify_edid_block_checksum(ext);
    let revision = ext[1];
    let dtd_offset = ext[2] as usize;
    let flags = ext[3];
    let underscan = (flags & 0x80) != 0;
    let basic_audio = (flags & 0x40) != 0;
    let ycbcr_444 = (flags & 0x20) != 0;
    let ycbcr_422 = (flags & 0x10) != 0;
    let native_dtd_count = flags & 0x0F;

    let mut audio_descriptors = Vec::new();
    let mut video_descriptors = Vec::new();
    let mut vendor_specific = Vec::new();
    let mut speaker_allocation_mask = None;
    let mut extended_tags = Vec::new();

    if dtd_offset > 4 {
        let end = dtd_offset.min(127);
        let mut pos = 4usize;
        while pos < end {
            let hdr = ext[pos];
            let block_tag = (hdr >> 5) & 0x07;
            let len = (hdr & 0x1F) as usize;
            let payload_start = pos + 1;
            let payload_end = (payload_start + len).min(end);
            if payload_start > end {
                break;
            }
            let payload = &ext[payload_start..payload_end];
            match block_tag {
                1 => {
                    for sad in payload.as_chunks::<3>().0 {
                        let format_code = (sad[0] >> 3) & 0x0F;
                        let max_channels = (sad[0] & 0x07) + 1;
                        let sample_rates_mask = sad[1];
                        audio_descriptors.push(CeaAudioDescriptor {
                            format_code,
                            format_name: cea_audio_format_name(format_code).to_string(),
                            max_channels,
                            sample_rates_mask,
                        });
                    }
                }
                2 => {
                    for &b in payload {
                        let vic = b & 0x7F;
                        let native = (b & 0x80) != 0;
                        video_descriptors.push(CeaVideoDescriptor {
                            vic,
                            native,
                            label: cea_vic_label(vic),
                        });
                    }
                }
                3 if payload.len() >= 3 => {
                    let ieee_oui = format!("{:02X}-{:02X}-{:02X}", payload[2], payload[1], payload[0]);
                    let (vendor_name, max_tmds_mhz) = match [payload[0], payload[1], payload[2]] {
                        [0x03, 0x0C, 0x00] => ("HDMI 1.4 Licensing LLC".to_string(), None),
                        [0xD8, 0x5D, 0xC4] => (
                            "HDMI Forum 2.0/2.1".to_string(),
                            payload.get(4).map(|&r| u32::from(r) * 5),
                        ),
                        _ => ("Vendor Specific".to_string(), None),
                    };
                    vendor_specific.push(CeaVendorSpecific {
                        ieee_oui,
                        vendor_name,
                        max_tmds_mhz,
                        raw_payload: payload.to_vec(),
                    });
                }
                4 if payload.len() >= 3 => {
                    let mask = u32::from(payload[0]) | (u32::from(payload[1]) << 8) | (u32::from(payload[2]) << 16);
                    speaker_allocation_mask = Some(mask);
                }
                7 if !payload.is_empty() => {
                    extended_tags.push(payload[0]);
                }
                _ => {}
            }
            pos = payload_start + len;
        }
    }

    let mut detailed_timings = Vec::new();
    if dtd_offset >= 4 {
        let mut pos = dtd_offset;
        while pos + 18 <= 127 {
            if let Some(dtd) = decode_dtd(&ext[pos..pos + 18]) {
                detailed_timings.push(dtd);
                pos += 18;
            } else {
                break;
            }
        }
    }

    Cea861Extension {
        revision,
        checksum_valid,
        underscan,
        basic_audio,
        ycbcr_444,
        ycbcr_422,
        native_dtd_count,
        audio_descriptors,
        video_descriptors,
        vendor_specific,
        speaker_allocation_mask,
        extended_tags,
        detailed_timings,
    }
}

pub fn parse_edid_bytes(bytes: &[u8]) -> WinctlResult<ParsedEdid> {
    if bytes.len() < 128 || !bytes.len().is_multiple_of(128) {
        return Err(WinctlError::ParseError(format!(
            "EDID payload length {} invalid (must be >= 128 and a multiple of 128 bytes)",
            bytes.len()
        )));
    }
    if bytes[0..8] != EDID_HEADER_MAGIC {
        return Err(WinctlError::ParseError(
            "invalid EDID header magic (expected 00 FF FF FF FF FF FF 00)".to_string(),
        ));
    }

    let b = &bytes[0..128];
    let base_checksum_valid = verify_edid_block_checksum(b);
    let manufacturer_id = decode_pnp_id(b[8], b[9]);
    let product_code = u16::from_le_bytes([b[10], b[11]]);
    let product_code_hex = format!("0x{product_code:04X}");
    let serial_number_u32 = u32::from_le_bytes([b[12], b[13], b[14], b[15]]);
    let manufacture_year = 1990u16 + u16::from(b[17]);
    let (manufacture_week, is_model_year) = if b[16] == 0xFF {
        (None, true)
    } else if b[16] == 0 {
        (None, false)
    } else {
        (Some(b[16]), false)
    };

    let ver_major = b[18];
    let ver_minor = b[19];
    let edid_version = format!("{ver_major}.{ver_minor}");
    let is_14_plus = ver_major > 1 || (ver_major == 1 && ver_minor >= 4);

    let is_digital = (b[20] & 0x80) != 0;
    let video_input = if is_digital {
        if is_14_plus {
            let depth_bits = (b[20] >> 4) & 0x07;
            let color_bit_depth = match depth_bits {
                1 => Some(6),
                2 => Some(8),
                3 => Some(10),
                4 => Some(12),
                5 => Some(14),
                6 => Some(16),
                _ => None,
            };
            let iface = match b[20] & 0x0F {
                1 => "DVI",
                2 => "HDMI-a",
                3 => "HDMI-b",
                4 => "MDDI",
                5 => "DisplayPort",
                _ => "Undefined",
            };
            EdidVideoInput {
                is_digital: true,
                color_bit_depth,
                digital_interface: Some(iface.to_string()),
                dfp_1x_compatible: None,
                analog_signal_level: None,
            }
        } else {
            EdidVideoInput {
                is_digital: true,
                color_bit_depth: None,
                digital_interface: None,
                dfp_1x_compatible: Some((b[20] & 0x01) != 0),
                analog_signal_level: None,
            }
        }
    } else {
        EdidVideoInput {
            is_digital: false,
            color_bit_depth: None,
            digital_interface: None,
            dfp_1x_compatible: None,
            analog_signal_level: Some((b[20] >> 5) & 0x03),
        }
    };

    let (width_cm, height_cm, diagonal_inches, aspect_ratio_hint) = if b[21] > 0 && b[22] > 0 {
        let w = f32::from(b[21]);
        let h = f32::from(b[22]);
        let diag = (w * w + h * h).sqrt() / 2.54;
        (
            Some(b[21]),
            Some(b[22]),
            Some((diag * 10.0).round() / 10.0),
            Some(((w / h) * 100.0).round() / 100.0),
        )
    } else if is_14_plus && b[21] > 0 && b[22] == 0 {
        (None, None, None, Some((f32::from(b[21]) + 99.0) / 100.0))
    } else if is_14_plus && b[21] == 0 && b[22] > 0 {
        (None, None, None, Some(100.0 / (f32::from(b[22]) + 99.0)))
    } else {
        (None, None, None, None)
    };

    let gamma = if b[23] == 0xFF {
        None
    } else {
        Some(((f32::from(b[23]) + 100.0) / 100.0 * 100.0).round() / 100.0)
    };

    let standby = (b[24] & 0x80) != 0;
    let suspend = (b[24] & 0x40) != 0;
    let active_off = (b[24] & 0x20) != 0;
    let srgb_default = (b[24] & 0x04) != 0;
    let preferred_timing_mode = (b[24] & 0x02) != 0;
    let continuous_freq = (b[24] & 0x01) != 0;

    let rx = ((u16::from(b[27]) << 2) | u16::from((b[25] >> 6) & 0x03)) as f32 / 1024.0;
    let ry = ((u16::from(b[28]) << 2) | u16::from((b[25] >> 4) & 0x03)) as f32 / 1024.0;
    let gx = ((u16::from(b[29]) << 2) | u16::from((b[25] >> 2) & 0x03)) as f32 / 1024.0;
    let gy = ((u16::from(b[30]) << 2) | u16::from(b[25] & 0x03)) as f32 / 1024.0;
    let bx = ((u16::from(b[31]) << 2) | u16::from((b[26] >> 6) & 0x03)) as f32 / 1024.0;
    let by = ((u16::from(b[32]) << 2) | u16::from((b[26] >> 4) & 0x03)) as f32 / 1024.0;
    let wx = ((u16::from(b[33]) << 2) | u16::from((b[26] >> 2) & 0x03)) as f32 / 1024.0;
    let wy = ((u16::from(b[34]) << 2) | u16::from(b[26] & 0x03)) as f32 / 1024.0;
    let chromaticity = EdidChromaticity {
        red_x: rx,
        red_y: ry,
        green_x: gx,
        green_y: gy,
        blue_x: bx,
        blue_y: by,
        white_x: wx,
        white_y: wy,
    };

    let established_timings = decode_established_timings(b);
    let standard_timings = decode_standard_timings(b, ver_major, ver_minor);

    let mut detailed_timings = Vec::new();
    let mut monitor_name = None;
    let mut ascii_serial = None;
    let mut unspecified_texts = Vec::new();
    let mut range_limits = None;

    for desc in b[54..126].as_chunks::<18>().0 {
        if let Some(dtd) = decode_dtd(desc) {
            detailed_timings.push(dtd);
        } else if desc[0] == 0 && desc[1] == 0 && desc[2] == 0 {
            match desc[3] {
                0xFC => {
                    let text = decode_edid_ascii(&desc[5..18]);
                    if !text.is_empty() {
                        monitor_name = Some(text);
                    }
                }
                0xFF => {
                    let text = decode_edid_ascii(&desc[5..18]);
                    if !text.is_empty() {
                        ascii_serial = Some(text);
                    }
                }
                0xFE => {
                    let text = decode_edid_ascii(&desc[5..18]);
                    if !text.is_empty() {
                        unspecified_texts.push(text);
                    }
                }
                0xFD => {
                    range_limits = Some(EdidRangeLimits {
                        min_v_hz: desc[5],
                        max_v_hz: desc[6],
                        min_h_khz: desc[7],
                        max_h_khz: desc[8],
                        max_pixel_clock_mhz: u16::from(desc[9]) * 10,
                    });
                }
                _ => {}
            }
        }
    }

    let extension_count = b[126];
    let available_exts = (bytes.len() / 128).saturating_sub(1);
    let parse_exts = usize::from(extension_count).min(available_exts);
    let mut cea_extensions = Vec::new();
    let mut all_checksums_valid = base_checksum_valid && usize::from(extension_count) <= available_exts;
    for i in 0..parse_exts {
        let start = (i + 1) * 128;
        let ext_block = &bytes[start..start + 128];
        if !verify_edid_block_checksum(ext_block) {
            all_checksums_valid = false;
        }
        if ext_block[0] == 0x02 {
            cea_extensions.push(parse_cea861_extension(ext_block));
        }
    }

    Ok(ParsedEdid {
        manufacturer_id,
        product_code,
        product_code_hex,
        serial_number_u32,
        manufacture_week,
        manufacture_year,
        is_model_year,
        edid_version,
        video_input,
        width_cm,
        height_cm,
        diagonal_inches,
        aspect_ratio_hint,
        gamma,
        standby,
        suspend,
        active_off,
        srgb_default,
        preferred_timing_mode,
        continuous_freq,
        chromaticity,
        established_timings,
        standard_timings,
        detailed_timings,
        monitor_name,
        ascii_serial,
        unspecified_texts,
        range_limits,
        extension_count,
        base_checksum_valid,
        all_checksums_valid,
        cea_extensions,
        raw_len: bytes.len(),
    })
}

pub fn parse_edid_input(input: &[u8]) -> WinctlResult<Vec<u8>> {
    if input.len() >= 128 && input[0..8] == EDID_HEADER_MAGIC {
        return Ok(input.to_vec());
    }
    let text = std::str::from_utf8(input)
        .map_err(|_| WinctlError::ParseError("binary input does not start with EDID magic header".to_string()))?;
    parse_edid_hex_text(text)
}

pub fn parse_edid_hex_text(text: &str) -> WinctlResult<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut cleaned = String::new();

    for raw_line in text.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with('#')
            || trimmed.starts_with(';')
            || trimmed.starts_with("Windows Registry Editor")
            || trimmed.starts_with('[')
        {
            continue;
        }
        let line_no_cont = trimmed.trim_end_matches('\\').trim();
        let lower_line = line_no_cont.to_ascii_lowercase();
        let body = if let Some(idx) = lower_line.find("hex:") {
            &line_no_cont[idx + 4..]
        } else if let Some(idx) = lower_line.find("hex(3):") {
            &line_no_cont[idx + 7..]
        } else if let Some((prefix, rest)) = line_no_cont.split_once(':') {
            let p = prefix.trim().trim_start_matches("0x").trim_start_matches("0X");
            if !p.is_empty()
                && p.len() <= 10
                && p.chars().all(|c| c.is_ascii_hexdigit())
                && rest.starts_with(char::is_whitespace)
                && !rest.trim_start().is_empty()
            {
                // Hex dump offset line like "00000000: 00 FF ... | ........"
                let mut hex_part = rest;
                if let Some((before_pipe, _)) = hex_part.split_once('|') {
                    hex_part = before_pipe;
                } else if hex_part.len() > 50 {
                    // Strip trailing ASCII column separated by 2+ spaces
                    if let Some(gap) = hex_part.find("  ") {
                        let candidate = &hex_part[..gap];
                        if candidate.trim().len() >= 16 {
                            hex_part = candidate;
                        }
                    }
                }
                hex_part
            } else {
                line_no_cont
            }
        } else {
            line_no_cont
        };
        cleaned.push_str(body);
        cleaned.push(' ');
    }

    let tokens: Vec<&str> = cleaned
        .split(|c: char| c.is_whitespace() || c == ',' || c == '-' || c == ':')
        .filter(|t| !t.is_empty())
        .collect();

    let single_stripped = if tokens.len() == 1 {
        Some(
            tokens[0]
                .strip_prefix("0x")
                .or_else(|| tokens[0].strip_prefix("0X"))
                .unwrap_or(tokens[0]),
        )
    } else {
        None
    };

    if let Some(s) = single_stripped
        && s.len() >= 256
        && s.len().is_multiple_of(2)
    {
        if !s.is_ascii() || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(WinctlError::ParseError(
                "non-ASCII or non-hex character in single-token EDID hex input".to_string(),
            ));
        }
        for pair in s.as_bytes().as_chunks::<2>().0 {
            let hex_pair = std::str::from_utf8(pair)
                .map_err(|_| WinctlError::ParseError("invalid UTF-8 in EDID hex input".to_string()))?;
            let b = u8::from_str_radix(hex_pair, 16)
                .map_err(|_| WinctlError::ParseError(format!("invalid hex byte '{hex_pair}' in EDID text")))?;
            bytes.push(b);
        }
    } else {
        for tok in tokens {
            let stripped = tok.strip_prefix("0x").or_else(|| tok.strip_prefix("0X")).unwrap_or(tok);
            if stripped.len() == 2 && stripped.chars().all(|c| c.is_ascii_hexdigit()) {
                let b = u8::from_str_radix(stripped, 16)
                    .map_err(|_| WinctlError::ParseError(format!("invalid hex byte '{tok}'")))?;
                bytes.push(b);
            } else if stripped.len() > 2
                && stripped.len().is_multiple_of(2)
                && stripped.chars().all(|c| c.is_ascii_hexdigit())
            {
                for pair in stripped.as_bytes().as_chunks::<2>().0 {
                    let hex_pair = std::str::from_utf8(pair)
                        .map_err(|_| WinctlError::ParseError("invalid UTF-8 in EDID hex input".to_string()))?;
                    let b = u8::from_str_radix(hex_pair, 16)
                        .map_err(|_| WinctlError::ParseError(format!("invalid hex byte '{hex_pair}'")))?;
                    bytes.push(b);
                }
            } else {
                return Err(WinctlError::ParseError(format!(
                    "unexpected token '{tok}' in EDID hex input"
                )));
            }
        }
    }

    if bytes.len() < 128 {
        return Err(WinctlError::ParseError(format!(
            "parsed EDID hex input only yielded {} bytes (need >= 128)",
            bytes.len()
        )));
    }
    Ok(bytes)
}

pub fn format_edid_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ")
}

pub fn edid_table_schema() -> TableSchema {
    TableSchema::new(
        "display_edid",
        "VESA EDID 1.3/1.4 & CEA-861 Records",
        vec![
            ColumnDef::new("registry_path", "Registry / Source Path", ColumnType::Text).with_width_hint(260),
            ColumnDef::new("manufacturer", "Manufacturer", ColumnType::Text).with_width_hint(100),
            ColumnDef::new("model_name", "Model Name", ColumnType::Text).with_width_hint(180),
            ColumnDef::new("serial", "Serial", ColumnType::Text).with_width_hint(140),
            ColumnDef::new("edid_version", "EDID Ver", ColumnType::Text).with_width_hint(80),
            ColumnDef::new("physical_size_mm", "Physical Size", ColumnType::Text).with_width_hint(120),
            ColumnDef::new("supported_timings", "Supported Timings", ColumnType::Text).with_width_hint(260),
            ColumnDef::new("checksum_valid", "Checksum OK", ColumnType::Bool).with_width_hint(90),
            ColumnDef::new("raw_hex", "Raw Hex", ColumnType::Hex).with_width_hint(240),
        ],
    )
}

pub fn edid_entry_to_row(id: u64, entry: &RawEdidEntry) -> WinctlResult<Row> {
    let parsed = parse_edid_bytes(&entry.raw_bytes)?;
    let model = parsed
        .monitor_name
        .clone()
        .unwrap_or_else(|| format!("{} {}", parsed.manufacturer_id, parsed.product_code_hex));
    let serial = parsed
        .ascii_serial
        .clone()
        .unwrap_or_else(|| format!("{:#010X}", parsed.serial_number_u32));
    let phys_size = match (parsed.width_cm, parsed.height_cm) {
        (Some(w), Some(h)) => format!("{}x{} mm", u16::from(w) * 10, u16::from(h) * 10),
        _ => "Unknown".to_string(),
    };
    let timings = parsed.summary_timings();
    let detail = serde_json::json!({
        "registry_path": entry.registry_path,
        "monitor_id": entry.monitor_id,
        "instance_id": entry.instance_id,
        "active": entry.active,
        "parsed": parsed,
    });

    Ok(Row::new(
        id,
        vec![
            CellValue::Text(entry.registry_path.clone()),
            CellValue::Text(parsed.manufacturer_id),
            CellValue::Text(model),
            CellValue::Text(serial),
            CellValue::Text(parsed.edid_version),
            CellValue::Text(phys_size),
            CellValue::Text(timings),
            CellValue::Bool(parsed.all_checksums_valid),
            CellValue::Hex(entry.raw_bytes.clone()),
        ],
    )
    .with_raw_bytes(entry.raw_bytes.clone())
    .with_detail_json(detail))
}

pub fn edid_table_from_bytes(source_label: &str, raw_bytes: &[u8], req: &TableQuery) -> WinctlResult<TableData> {
    let entry = RawEdidEntry {
        registry_path: source_label.to_string(),
        monitor_id: "OFFLINE".to_string(),
        instance_id: "0".to_string(),
        device_interface_id: String::new(),
        active: false,
        raw_bytes: raw_bytes.to_vec(),
    };
    let row = edid_entry_to_row(1, &entry)?;
    Ok(apply_query(&edid_table_schema(), &[row], req)?)
}

pub fn edid_table_from_hex(source_label: &str, hex_or_reg: &str, req: &TableQuery) -> WinctlResult<TableData> {
    let bytes = parse_edid_hex_text(hex_or_reg)?;
    edid_table_from_bytes(source_label, &bytes, req)
}

pub struct EdidTableProvider {
    backend: Arc<dyn DisplayBackend>,
}

impl EdidTableProvider {
    pub fn new(backend: Arc<dyn DisplayBackend>) -> Self {
        Self { backend }
    }
}

impl TableProvider for EdidTableProvider {
    fn tool_id(&self) -> &'static str {
        "display_edid"
    }

    fn description(&self) -> &'static str {
        "Parse and inspect VESA EDID 1.3/1.4 and CEA-861 monitor descriptor blocks"
    }

    fn schema(&self) -> TableSchema {
        edid_table_schema()
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn query(&self, req: &TableQuery) -> Result<TableData, TableError> {
        let records = self
            .backend
            .list_edid_records()
            .map_err(|e| TableError::Provider(e.to_string()))?;
        let mut rows = Vec::new();
        for (idx, rec) in records.iter().enumerate() {
            match edid_entry_to_row(idx as u64 + 1, rec) {
                Ok(row) => rows.push(row),
                Err(err) => {
                    rows.push(
                        Row::new(
                            idx as u64 + 1,
                            vec![
                                CellValue::Text(rec.registry_path.clone()),
                                CellValue::Text("ERR".to_string()),
                                CellValue::Text(rec.monitor_id.clone()),
                                CellValue::Text(rec.instance_id.clone()),
                                CellValue::Text(String::new()),
                                CellValue::Text(String::new()),
                                CellValue::Text(err.to_string()),
                                CellValue::Bool(false),
                                CellValue::Hex(rec.raw_bytes.clone()),
                            ],
                        )
                        .with_raw_bytes(rec.raw_bytes.clone()),
                    );
                }
            }
        }
        apply_query(&self.schema(), &rows, req)
    }
}
