use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use winctl2rust_core::{WinctlError, WinctlResult};

const MAX_TTCF_FONTS: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SfntFormat {
    TrueType,
    OpenTypeCff,
    AppleTrueType,
    Type1Sfnt,
}

impl SfntFormat {
    pub fn label(self) -> &'static str {
        match self {
            Self::TrueType => "TrueType",
            Self::OpenTypeCff => "OpenType CFF",
            Self::AppleTrueType => "Apple TrueType",
            Self::Type1Sfnt => "Type1 sfnt",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NameId {
    Copyright,
    Family,
    Subfamily,
    UniqueId,
    FullName,
    Version,
    PostScriptName,
    Trademark,
    Manufacturer,
    Designer,
    Description,
    VendorUrl,
    DesignerUrl,
    LicenseDescription,
    LicenseUrl,
    TypographicFamily,
    TypographicSubfamily,
    Other(u16),
}

impl NameId {
    pub fn from_u16(id: u16) -> Self {
        match id {
            0 => Self::Copyright,
            1 => Self::Family,
            2 => Self::Subfamily,
            3 => Self::UniqueId,
            4 => Self::FullName,
            5 => Self::Version,
            6 => Self::PostScriptName,
            7 => Self::Trademark,
            8 => Self::Manufacturer,
            9 => Self::Designer,
            10 => Self::Description,
            11 => Self::VendorUrl,
            12 => Self::DesignerUrl,
            13 => Self::LicenseDescription,
            14 => Self::LicenseUrl,
            16 => Self::TypographicFamily,
            17 => Self::TypographicSubfamily,
            other => Self::Other(other),
        }
    }

    pub fn as_u16(self) -> u16 {
        match self {
            Self::Copyright => 0,
            Self::Family => 1,
            Self::Subfamily => 2,
            Self::UniqueId => 3,
            Self::FullName => 4,
            Self::Version => 5,
            Self::PostScriptName => 6,
            Self::Trademark => 7,
            Self::Manufacturer => 8,
            Self::Designer => 9,
            Self::Description => 10,
            Self::VendorUrl => 11,
            Self::DesignerUrl => 12,
            Self::LicenseDescription => 13,
            Self::LicenseUrl => 14,
            Self::TypographicFamily => 16,
            Self::TypographicSubfamily => 17,
            Self::Other(id) => id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableRecord {
    pub tag: [u8; 4],
    pub checksum: u32,
    pub offset: u32,
    pub length: u32,
}

impl TableRecord {
    pub fn tag_str(&self) -> String {
        String::from_utf8_lossy(&self.tag).into_owned()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NameRecordEntry {
    pub platform_id: u16,
    pub encoding_id: u16,
    pub language_id: u16,
    pub language_tag: Option<String>,
    pub name_id: NameId,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedNameTable {
    pub format: u16,
    pub lang_tags: Vec<String>,
    pub records: Vec<NameRecordEntry>,
}

impl ParsedNameTable {
    pub fn resolve_name(&self, target_id: u16) -> Option<&str> {
        let mut best: Option<(u8, &str)> = None;
        for rec in &self.records {
            if rec.name_id.as_u16() != target_id || rec.value.is_empty() {
                continue;
            }
            let rank = match (rec.platform_id, rec.encoding_id, rec.language_id) {
                (3, 10, 0x0409) => 0,
                (3, 1, 0x0409) => 1,
                (3, 1 | 10, lang)
                    if (lang & 0x00FF) == 0x09
                        || (lang >= 0x8000
                            && rec
                                .language_tag
                                .as_deref()
                                .is_some_and(|t| t.to_ascii_lowercase().starts_with("en"))) =>
                {
                    2
                }
                (3, _, _) => 3,
                (0, _, _) => 4,
                (1, 0, 0) => 5,
                (1, 0, _) => 6,
                _ => 7,
            };
            if best.is_none_or(|(best_rank, _)| rank < best_rank) {
                best = Some((rank, rec.value.as_str()));
            }
        }
        best.map(|(_, v)| v)
    }

    pub fn effective_family(&self) -> Option<&str> {
        self.resolve_name(16).or_else(|| self.resolve_name(1))
    }

    pub fn effective_subfamily(&self) -> Option<&str> {
        self.resolve_name(17).or_else(|| self.resolve_name(2))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedOs2Table {
    pub version: u16,
    pub x_avg_char_width: i16,
    pub us_weight_class: u16,
    pub weight_label: String,
    pub us_width_class: u16,
    pub width_label: String,
    pub fs_type: u16,
    pub panose: [u8; 10],
    pub ach_vend_id: String,
    pub fs_selection: u16,
    pub s_typo_ascender: i16,
    pub s_typo_descender: i16,
    pub s_typo_line_gap: i16,
    pub ul_code_page_range1: Option<u32>,
    pub ul_code_page_range2: Option<u32>,
}

impl ParsedOs2Table {
    pub fn panose_formatted(&self) -> String {
        self.panose
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join("-")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedFontFace {
    pub face_index: u32,
    pub format: SfntFormat,
    pub tables: Vec<TableRecord>,
    pub name: Option<ParsedNameTable>,
    pub os2: Option<ParsedOs2Table>,
}

impl ParsedFontFace {
    pub fn family(&self) -> String {
        self.name
            .as_ref()
            .and_then(ParsedNameTable::effective_family)
            .unwrap_or("")
            .to_string()
    }

    pub fn subfamily(&self) -> String {
        self.name
            .as_ref()
            .and_then(ParsedNameTable::effective_subfamily)
            .unwrap_or("")
            .to_string()
    }

    pub fn full_name(&self) -> String {
        self.name
            .as_ref()
            .and_then(|n| n.resolve_name(4))
            .unwrap_or("")
            .to_string()
    }

    pub fn postscript_name(&self) -> String {
        self.name
            .as_ref()
            .and_then(|n| n.resolve_name(6))
            .unwrap_or("")
            .to_string()
    }

    pub fn version(&self) -> String {
        self.name
            .as_ref()
            .and_then(|n| n.resolve_name(5))
            .unwrap_or("")
            .to_string()
    }
}

pub fn calc_sfnt_checksum(slice: &[u8], is_head: bool) -> u32 {
    let mut sum = 0u32;
    let mut idx = 0usize;
    while idx < slice.len() {
        let mut buf = [0u8; 4];
        let take = (slice.len() - idx).min(4);
        buf[..take].copy_from_slice(&slice[idx..idx + take]);
        if is_head && idx == 8 {
            buf = [0u8; 4];
        }
        sum = sum.wrapping_add(u32::from_be_bytes(buf));
        idx += 4;
    }
    sum
}

pub fn parse_font_bytes(bytes: &[u8]) -> WinctlResult<Vec<ParsedFontFace>> {
    if bytes.len() < 12 {
        return Err(WinctlError::ParseError(format!(
            "font data too short: {} bytes (need >= 12)",
            bytes.len()
        )));
    }
    if &bytes[0..4] == b"ttcf" {
        let major = read_u16_be(bytes, 4)?;
        let _minor = read_u16_be(bytes, 6)?;
        if major != 1 && major != 2 {
            return Err(WinctlError::ParseError(format!(
                "unsupported ttcf major version {major}"
            )));
        }
        let num_fonts = read_u32_be(bytes, 8)? as usize;
        if num_fonts == 0 || num_fonts > MAX_TTCF_FONTS {
            return Err(WinctlError::ParseError(format!("invalid ttcf numFonts {num_fonts}")));
        }
        let offsets_bytes = num_fonts
            .checked_mul(4)
            .and_then(|n| 12usize.checked_add(n))
            .ok_or_else(|| WinctlError::ParseError("ttcf offset table overflow".to_string()))?;
        if offsets_bytes > bytes.len() {
            return Err(WinctlError::ParseError(format!(
                "ttcf offset table truncated ({offsets_bytes} > {})",
                bytes.len()
            )));
        }
        let mut face_offsets = Vec::with_capacity(num_fonts);
        let mut face_dirs: Vec<(usize, usize)> = Vec::with_capacity(num_fonts);
        for i in 0..num_fonts {
            let sfnt_offset = read_u32_be(bytes, 12 + i * 4)? as usize;
            if sfnt_offset < offsets_bytes {
                return Err(WinctlError::ParseError(format!(
                    "ttcf face {i} offset {sfnt_offset} overlaps ttcf header (0..{offsets_bytes})"
                )));
            }
            let header_end = sfnt_offset
                .checked_add(12)
                .ok_or_else(|| WinctlError::ParseError("sfnt header offset overflow".to_string()))?;
            if header_end > bytes.len() {
                return Err(WinctlError::ParseError(format!(
                    "sfnt offset {sfnt_offset} exceeds buffer len {}",
                    bytes.len()
                )));
            }
            let num_tables = read_u16_be(bytes, sfnt_offset + 4)? as usize;
            let dir_bytes = num_tables
                .checked_mul(16)
                .ok_or_else(|| WinctlError::ParseError("sfnt table directory size overflow".to_string()))?;
            let dir_end = header_end
                .checked_add(dir_bytes)
                .ok_or_else(|| WinctlError::ParseError("sfnt directory end overflow".to_string()))?;
            if dir_end > bytes.len() {
                return Err(WinctlError::ParseError(format!(
                    "sfnt table directory truncated ({dir_end} > {})",
                    bytes.len()
                )));
            }
            for &(prev_s, prev_e) in &face_dirs {
                if sfnt_offset < prev_e && prev_s < dir_end {
                    return Err(WinctlError::ParseError(format!(
                        "ttcf face {i} directory ({sfnt_offset}..{dir_end}) overlaps another face directory ({prev_s}..{prev_e})"
                    )));
                }
            }
            face_dirs.push((sfnt_offset, dir_end));
            face_offsets.push(sfnt_offset);
        }
        let mut reserved = Vec::with_capacity(1 + face_dirs.len());
        reserved.push((0usize, offsets_bytes));
        reserved.extend(face_dirs);

        let mut faces = Vec::with_capacity(num_fonts);
        for (i, sfnt_offset) in face_offsets.into_iter().enumerate() {
            faces.push(parse_sfnt_at_with_reserved(bytes, sfnt_offset, i as u32, &reserved)?);
        }
        Ok(faces)
    } else {
        Ok(vec![parse_sfnt_at_with_reserved(bytes, 0, 0, &[])?])
    }
}

fn parse_sfnt_at_with_reserved(
    bytes: &[u8],
    sfnt_offset: usize,
    face_index: u32,
    extra_reserved: &[(usize, usize)],
) -> WinctlResult<ParsedFontFace> {
    let header_end = sfnt_offset
        .checked_add(12)
        .ok_or_else(|| WinctlError::ParseError("sfnt header offset overflow".to_string()))?;
    if header_end > bytes.len() {
        return Err(WinctlError::ParseError(format!(
            "sfnt offset {sfnt_offset} exceeds buffer len {}",
            bytes.len()
        )));
    }

    let magic = &bytes[sfnt_offset..sfnt_offset + 4];
    let format = match magic {
        [0x00, 0x01, 0x00, 0x00] => SfntFormat::TrueType,
        b"OTTO" => SfntFormat::OpenTypeCff,
        b"true" => SfntFormat::AppleTrueType,
        b"typ1" => SfntFormat::Type1Sfnt,
        other => {
            return Err(WinctlError::ParseError(format!(
                "unrecognized sfnt/ttcf magic {:02X?}",
                other
            )));
        }
    };

    let num_tables = read_u16_be(bytes, sfnt_offset + 4)? as usize;
    let dir_bytes = num_tables
        .checked_mul(16)
        .ok_or_else(|| WinctlError::ParseError("sfnt table directory size overflow".to_string()))?;
    let dir_end = header_end
        .checked_add(dir_bytes)
        .ok_or_else(|| WinctlError::ParseError("sfnt directory end overflow".to_string()))?;
    if dir_end > bytes.len() {
        return Err(WinctlError::ParseError(format!(
            "sfnt table directory truncated ({dir_end} > {})",
            bytes.len()
        )));
    }

    let mut tables = Vec::with_capacity(num_tables);
    let mut seen_tags = HashSet::with_capacity(num_tables);
    let mut spans: Vec<(usize, usize, [u8; 4])> = Vec::with_capacity(num_tables);

    for i in 0..num_tables {
        let rec_off = header_end + i * 16;
        let tag: [u8; 4] = bytes[rec_off..rec_off + 4]
            .try_into()
            .map_err(|_| WinctlError::ParseError("short table record tag".to_string()))?;
        if !seen_tags.insert(tag) {
            return Err(WinctlError::ParseError(format!(
                "duplicate sfnt table tag '{}'",
                String::from_utf8_lossy(&tag)
            )));
        }
        let checksum = read_u32_be(bytes, rec_off + 4)?;
        let offset = read_u32_be(bytes, rec_off + 8)?;
        let length = read_u32_be(bytes, rec_off + 12)?;
        let start = offset as usize;
        let end = start
            .checked_add(length as usize)
            .ok_or_else(|| WinctlError::ParseError("sfnt table span overflow".to_string()))?;
        if end > bytes.len() {
            return Err(WinctlError::ParseError(format!(
                "sfnt table '{}' truncated ({end} > {})",
                String::from_utf8_lossy(&tag),
                bytes.len()
            )));
        }
        if length > 0 && start < dir_end && end > sfnt_offset {
            return Err(WinctlError::ParseError(format!(
                "sfnt table '{}' overlaps table directory",
                String::from_utf8_lossy(&tag)
            )));
        }
        if length > 0 {
            for &(res_s, res_e) in extra_reserved {
                if start < res_e && res_s < end {
                    return Err(WinctlError::ParseError(format!(
                        "sfnt table '{}' overlaps reserved header/directory ({res_s}..{res_e})",
                        String::from_utf8_lossy(&tag)
                    )));
                }
            }
            for &(prev_s, prev_e, prev_tag) in &spans {
                if start < prev_e && prev_s < end {
                    return Err(WinctlError::ParseError(format!(
                        "sfnt table '{}' overlaps table '{}'",
                        String::from_utf8_lossy(&tag),
                        String::from_utf8_lossy(&prev_tag)
                    )));
                }
            }
            spans.push((start, end, tag));
        }

        tables.push(TableRecord {
            tag,
            checksum,
            offset,
            length,
        });
    }

    let name = match tables.iter().find(|t| &t.tag == b"name") {
        Some(rec) => {
            let s = rec.offset as usize;
            let e = s + rec.length as usize;
            Some(parse_name_table(&bytes[s..e])?)
        }
        None => None,
    };

    let os2 = match tables.iter().find(|t| &t.tag == b"OS/2") {
        Some(rec) => {
            let s = rec.offset as usize;
            let e = s + rec.length as usize;
            Some(parse_os2_table(&bytes[s..e])?)
        }
        None => None,
    };

    Ok(ParsedFontFace {
        face_index,
        format,
        tables,
        name,
        os2,
    })
}

pub fn parse_name_table(data: &[u8]) -> WinctlResult<ParsedNameTable> {
    if data.len() < 6 {
        return Err(WinctlError::ParseError(format!(
            "OpenType name table too short: {} bytes (need >= 6)",
            data.len()
        )));
    }
    let format = read_u16_be(data, 0)?;
    if format != 0 && format != 1 {
        return Err(WinctlError::ParseError(format!(
            "unsupported OpenType name table format {format}"
        )));
    }
    let count = read_u16_be(data, 2)? as usize;
    let storage_offset = read_u16_be(data, 4)? as usize;

    let records_end = count
        .checked_mul(12)
        .and_then(|n| 6usize.checked_add(n))
        .ok_or_else(|| WinctlError::ParseError("name table records size overflow".to_string()))?;
    if records_end > data.len() {
        return Err(WinctlError::ParseError(format!(
            "name table records exceed table length ({records_end} > {})",
            data.len()
        )));
    }

    let mut lang_tags = Vec::new();
    let header_end = if format == 1 {
        let lang_tag_count = read_u16_be(data, records_end)? as usize;
        let tags_end = lang_tag_count
            .checked_mul(4)
            .and_then(|n| records_end.checked_add(2)?.checked_add(n))
            .ok_or_else(|| WinctlError::ParseError("name table langTagCount overflow".to_string()))?;
        if tags_end > data.len() {
            return Err(WinctlError::ParseError(format!(
                "name table langTag records exceed table length ({tags_end} > {})",
                data.len()
            )));
        }
        for i in 0..lang_tag_count {
            let off = records_end + 2 + i * 4;
            let len = read_u16_be(data, off)? as usize;
            let str_off = read_u16_be(data, off + 2)? as usize;
            let s_start = storage_offset
                .checked_add(str_off)
                .ok_or_else(|| WinctlError::ParseError("langTag offset overflow".to_string()))?;
            let s_end = s_start
                .checked_add(len)
                .ok_or_else(|| WinctlError::ParseError("langTag end overflow".to_string()))?;
            if s_end > data.len() {
                return Err(WinctlError::ParseError("langTag string exceeds name table".to_string()));
            }
            lang_tags.push(decode_utf16_be(&data[s_start..s_end], "langTag")?);
        }
        tags_end
    } else {
        records_end
    };

    if storage_offset < header_end || storage_offset > data.len() {
        return Err(WinctlError::ParseError(format!(
            "invalid name table storageOffset {storage_offset} (header_end={header_end}, len={})",
            data.len()
        )));
    }

    let mut records = Vec::with_capacity(count);
    for i in 0..count {
        let rec_off = 6 + i * 12;
        let platform_id = read_u16_be(data, rec_off)?;
        let encoding_id = read_u16_be(data, rec_off + 2)?;
        let language_id = read_u16_be(data, rec_off + 4)?;
        let name_id_raw = read_u16_be(data, rec_off + 6)?;
        let length = read_u16_be(data, rec_off + 8)? as usize;
        let string_offset = read_u16_be(data, rec_off + 10)? as usize;

        let s_start = storage_offset
            .checked_add(string_offset)
            .ok_or_else(|| WinctlError::ParseError("name string offset overflow".to_string()))?;
        let s_end = s_start
            .checked_add(length)
            .ok_or_else(|| WinctlError::ParseError("name string end overflow".to_string()))?;
        if s_end > data.len() {
            return Err(WinctlError::ParseError(format!(
                "name record {i} string exceeds table ({s_end} > {})",
                data.len()
            )));
        }

        let slice = &data[s_start..s_end];
        let value = if platform_id == 0 || (platform_id == 3 && matches!(encoding_id, 0 | 1 | 10)) {
            decode_utf16_be(slice, "NameRecord")?
        } else if platform_id == 1 && encoding_id == 0 {
            decode_mac_roman(slice)
        } else {
            String::from_utf8_lossy(slice).trim_end_matches('\0').to_string()
        };

        let language_tag = if format == 1 && language_id >= 0x8000 {
            lang_tags.get((language_id - 0x8000) as usize).cloned()
        } else {
            None
        };

        records.push(NameRecordEntry {
            platform_id,
            encoding_id,
            language_id,
            language_tag,
            name_id: NameId::from_u16(name_id_raw),
            value,
        });
    }

    Ok(ParsedNameTable {
        format,
        lang_tags,
        records,
    })
}

pub fn parse_os2_table(data: &[u8]) -> WinctlResult<ParsedOs2Table> {
    if data.len() < 78 {
        return Err(WinctlError::ParseError(format!(
            "OpenType OS/2 table too short: {} bytes (need >= 78)",
            data.len()
        )));
    }
    let version = read_u16_be(data, 0)?;
    let min_len = match version {
        0 => 78,
        1 => 86,
        2..=4 => 96,
        _ => 100,
    };
    if data.len() < min_len {
        return Err(WinctlError::ParseError(format!(
            "OpenType OS/2 v{version} table too short: {} bytes (need >= {min_len})",
            data.len()
        )));
    }
    let x_avg_char_width = read_i16_be(data, 2)?;
    let us_weight_class = read_u16_be(data, 4)?;
    let us_width_class = read_u16_be(data, 6)?;
    let fs_type = read_u16_be(data, 8)?;
    let mut panose = [0u8; 10];
    panose.copy_from_slice(&data[0x20..0x2A]);
    let ach_vend_id = String::from_utf8_lossy(&data[0x3A..0x3E])
        .trim_end_matches('\0')
        .trim()
        .to_string();
    let fs_selection = read_u16_be(data, 0x3E)?;
    let s_typo_ascender = read_i16_be(data, 0x44)?;
    let s_typo_descender = read_i16_be(data, 0x46)?;
    let s_typo_line_gap = read_i16_be(data, 0x48)?;

    let (ul_code_page_range1, ul_code_page_range2) = if version >= 1 && data.len() >= 86 {
        (Some(read_u32_be(data, 0x4E)?), Some(read_u32_be(data, 0x52)?))
    } else {
        (None, None)
    };

    Ok(ParsedOs2Table {
        version,
        x_avg_char_width,
        us_weight_class,
        weight_label: weight_class_label(us_weight_class).to_string(),
        us_width_class,
        width_label: width_class_label(us_width_class).to_string(),
        fs_type,
        panose,
        ach_vend_id,
        fs_selection,
        s_typo_ascender,
        s_typo_descender,
        s_typo_line_gap,
        ul_code_page_range1,
        ul_code_page_range2,
    })
}

pub fn weight_class_label(weight: u16) -> &'static str {
    match weight {
        0..=149 => "Thin",
        150..=249 => "ExtraLight",
        250..=349 => "Light",
        350..=449 => "Regular",
        450..=549 => "Medium",
        550..=649 => "SemiBold",
        650..=749 => "Bold",
        750..=849 => "ExtraBold",
        _ => "Black",
    }
}

pub fn width_class_label(width: u16) -> &'static str {
    match width {
        1 => "UltraCondensed",
        2 => "ExtraCondensed",
        3 => "Condensed",
        4 => "SemiCondensed",
        5 => "Normal",
        6 => "SemiExpanded",
        7 => "Expanded",
        8 => "ExtraExpanded",
        9 => "UltraExpanded",
        _ => "Unknown",
    }
}

fn decode_utf16_be(slice: &[u8], context: &str) -> WinctlResult<String> {
    if !slice.len().is_multiple_of(2) {
        return Err(WinctlError::ParseError(format!(
            "{context} UTF-16BE string has odd byte length {}",
            slice.len()
        )));
    }
    let units: Vec<u16> = slice
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_be_bytes(*c))
        .collect();
    Ok(String::from_utf16_lossy(&units).trim_end_matches('\0').to_string())
}

pub fn decode_mac_roman(slice: &[u8]) -> String {
    const MAC_ROMAN_HIGH: [char; 128] = [
        '\u{00C4}', '\u{00C5}', '\u{00C7}', '\u{00C9}', '\u{00D1}', '\u{00D6}', '\u{00DC}', '\u{00E1}', '\u{00E0}',
        '\u{00E2}', '\u{00E4}', '\u{00E3}', '\u{00E5}', '\u{00E7}', '\u{00E9}', '\u{00E8}', '\u{00EA}', '\u{00EB}',
        '\u{00ED}', '\u{00EC}', '\u{00EE}', '\u{00EF}', '\u{00F1}', '\u{00F3}', '\u{00F2}', '\u{00F4}', '\u{00F6}',
        '\u{00F5}', '\u{00FA}', '\u{00F9}', '\u{00FB}', '\u{00FC}', '\u{2020}', '\u{00B0}', '\u{00A2}', '\u{00A3}',
        '\u{00A7}', '\u{2022}', '\u{00B6}', '\u{00DF}', '\u{00AE}', '\u{00A9}', '\u{2122}', '\u{00B4}', '\u{00A8}',
        '\u{2260}', '\u{00C6}', '\u{00D8}', '\u{221E}', '\u{00B1}', '\u{2264}', '\u{2265}', '\u{00A5}', '\u{00B5}',
        '\u{2202}', '\u{2211}', '\u{220F}', '\u{03C0}', '\u{222B}', '\u{00AA}', '\u{00BA}', '\u{03A9}', '\u{00E6}',
        '\u{00F8}', '\u{00BF}', '\u{00A1}', '\u{00AC}', '\u{221A}', '\u{0192}', '\u{2248}', '\u{2206}', '\u{00AB}',
        '\u{00BB}', '\u{2026}', '\u{00A0}', '\u{00C0}', '\u{00C3}', '\u{00D5}', '\u{0152}', '\u{0153}', '\u{2013}',
        '\u{2014}', '\u{201C}', '\u{201D}', '\u{2018}', '\u{2019}', '\u{00F7}', '\u{25CA}', '\u{00FF}', '\u{0178}',
        '\u{2044}', '\u{20AC}', '\u{2039}', '\u{203A}', '\u{FB01}', '\u{FB02}', '\u{2021}', '\u{00B7}', '\u{201A}',
        '\u{201E}', '\u{2030}', '\u{00C2}', '\u{00CA}', '\u{00C1}', '\u{00CB}', '\u{00C8}', '\u{00CD}', '\u{00CE}',
        '\u{00CF}', '\u{00CC}', '\u{00D3}', '\u{00D4}', '\u{F8FF}', '\u{00D2}', '\u{00DA}', '\u{00DB}', '\u{00D9}',
        '\u{0131}', '\u{02C6}', '\u{02DC}', '\u{00AF}', '\u{02D8}', '\u{02D9}', '\u{02DA}', '\u{00B8}', '\u{02DD}',
        '\u{02DB}', '\u{02C7}',
    ];
    let mut out = String::with_capacity(slice.len());
    for &b in slice {
        if b < 0x80 {
            if b != 0 {
                out.push(b as char);
            }
        } else {
            out.push(MAC_ROMAN_HIGH[(b - 0x80) as usize]);
        }
    }
    out
}

fn read_u16_be(bytes: &[u8], offset: usize) -> WinctlResult<u16> {
    let end = offset
        .checked_add(2)
        .ok_or_else(|| WinctlError::ParseError("offset overflow reading u16".to_string()))?;
    if end > bytes.len() {
        return Err(WinctlError::ParseError(format!(
            "unexpected EOF reading u16 at {offset} (len {})",
            bytes.len()
        )));
    }
    Ok(u16::from_be_bytes([bytes[offset], bytes[offset + 1]]))
}

fn read_i16_be(bytes: &[u8], offset: usize) -> WinctlResult<i16> {
    Ok(read_u16_be(bytes, offset)? as i16)
}

fn read_u32_be(bytes: &[u8], offset: usize) -> WinctlResult<u32> {
    let end = offset
        .checked_add(4)
        .ok_or_else(|| WinctlError::ParseError("offset overflow reading u32".to_string()))?;
    if end > bytes.len() {
        return Err(WinctlError::ParseError(format!(
            "unexpected EOF reading u32 at {offset} (len {})",
            bytes.len()
        )));
    }
    Ok(u32::from_be_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ]))
}
