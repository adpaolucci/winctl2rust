use std::fs;
use std::path::Path;
use std::sync::Arc;

use guap_mcp::{ActionCommand, ActionRiskLevel, McpToolError};
use guap_table::{
    CellValue, ColumnDef, ColumnType, Row, TableData, TableError, TableProvider, TableQuery, TableSchema, apply_query,
};
use serde::Deserialize;
use serde_json::{Value, json};
use winctl2rust_core::{WinctlError, WinctlResult, dry_run_preview, validate_r4_compliance};

use crate::backend::{DesktopBackend, RawVideoFrame, VideoPixelFormat, WebcamDeviceInfo, validate_safe_path};
use crate::clipboard::sha256_hex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StillImageFormat {
    Bmp,
    Ppm,
}

impl StillImageFormat {
    pub fn parse(raw: Option<&str>, path: &str) -> WinctlResult<Self> {
        if let Some(fmt) = raw.map(str::trim).filter(|s| !s.is_empty()) {
            return match fmt.to_ascii_lowercase().as_str() {
                "bmp" => Ok(Self::Bmp),
                "ppm" => Ok(Self::Ppm),
                other => Err(WinctlError::InvalidInput(format!(
                    "unsupported webcam capture format '{other}' (expected bmp or ppm)"
                ))),
            };
        }
        if Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("ppm"))
        {
            Ok(Self::Ppm)
        } else {
            Ok(Self::Bmp)
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Bmp => "bmp",
            Self::Ppm => "ppm",
        }
    }
}

pub fn expected_frame_byte_len(width: u32, height: u32, fmt: VideoPixelFormat) -> WinctlResult<usize> {
    if width == 0 || height == 0 {
        return Err(WinctlError::InvalidInput(
            "video frame width and height must be positive".to_string(),
        ));
    }
    let w = width as usize;
    let h = height as usize;
    let pixels = w
        .checked_mul(h)
        .ok_or_else(|| WinctlError::InvalidInput("video frame pixel count overflow".to_string()))?;

    match fmt {
        VideoPixelFormat::Rgb24 => pixels
            .checked_mul(3)
            .ok_or_else(|| WinctlError::InvalidInput("RGB24 frame byte length overflow".to_string())),
        VideoPixelFormat::Bgra32 => pixels
            .checked_mul(4)
            .ok_or_else(|| WinctlError::InvalidInput("BGRA32 frame byte length overflow".to_string())),
        VideoPixelFormat::Yuy2 => {
            if !width.is_multiple_of(2) {
                return Err(WinctlError::InvalidInput(format!(
                    "YUY2 requires even width (got {width})"
                )));
            }
            pixels
                .checked_mul(2)
                .ok_or_else(|| WinctlError::InvalidInput("YUY2 frame byte length overflow".to_string()))
        }
        VideoPixelFormat::Nv12 => {
            if !width.is_multiple_of(2) || !height.is_multiple_of(2) {
                return Err(WinctlError::InvalidInput(format!(
                    "NV12 requires even width and height (got {width}x{height})"
                )));
            }
            pixels
                .checked_mul(3)
                .map(|v| v / 2)
                .ok_or_else(|| WinctlError::InvalidInput("NV12 frame byte length overflow".to_string()))
        }
    }
}

pub fn validate_video_frame(frame: &RawVideoFrame) -> WinctlResult<usize> {
    let expected = expected_frame_byte_len(frame.width, frame.height, frame.pixel_format)?;
    if frame.data.len() < expected {
        return Err(WinctlError::ParseError(format!(
            "{} {}x{} frame buffer truncated: got {} bytes, need at least {expected}",
            frame.pixel_format.label(),
            frame.width,
            frame.height,
            frame.data.len()
        )));
    }
    Ok(expected)
}

#[inline]
fn bt601_yuv_to_rgb(y: u8, u: u8, v: u8) -> [u8; 3] {
    let c = i32::from(y) - 16;
    let d = i32::from(u) - 128;
    let e = i32::from(v) - 128;
    let r = ((298 * c + 409 * e + 128) >> 8).clamp(0, 255) as u8;
    let g = ((298 * c - 100 * d - 208 * e + 128) >> 8).clamp(0, 255) as u8;
    let b = ((298 * c + 516 * d + 128) >> 8).clamp(0, 255) as u8;
    [r, g, b]
}

pub fn frame_to_rgb24(frame: &RawVideoFrame) -> WinctlResult<Vec<u8>> {
    let expected = validate_video_frame(frame)?;
    let w = frame.width as usize;
    let h = frame.height as usize;
    let pixel_count = w * h;
    let mut rgb = Vec::with_capacity(pixel_count * 3);

    match frame.pixel_format {
        VideoPixelFormat::Rgb24 => {
            rgb.extend_from_slice(&frame.data[..expected]);
        }
        VideoPixelFormat::Bgra32 => {
            for px in frame.data[..expected].as_chunks::<4>().0 {
                rgb.push(px[2]);
                rgb.push(px[1]);
                rgb.push(px[0]);
            }
        }
        VideoPixelFormat::Yuy2 => {
            for macro_px in frame.data[..expected].as_chunks::<4>().0 {
                let y0 = macro_px[0];
                let u = macro_px[1];
                let y1 = macro_px[2];
                let v = macro_px[3];
                rgb.extend_from_slice(&bt601_yuv_to_rgb(y0, u, v));
                rgb.extend_from_slice(&bt601_yuv_to_rgb(y1, u, v));
            }
        }
        VideoPixelFormat::Nv12 => {
            let y_plane = &frame.data[..pixel_count];
            let uv_plane = &frame.data[pixel_count..expected];
            for y in 0..h {
                let uv_row = (y / 2) * w;
                for x in 0..w {
                    let y_val = y_plane[y * w + x];
                    let uv_idx = uv_row + (x & !1);
                    let u_val = uv_plane[uv_idx];
                    let v_val = uv_plane[uv_idx + 1];
                    rgb.extend_from_slice(&bt601_yuv_to_rgb(y_val, u_val, v_val));
                }
            }
        }
    }

    Ok(rgb)
}

pub fn encode_bmp_24(frame: &RawVideoFrame) -> WinctlResult<Vec<u8>> {
    let rgb = frame_to_rgb24(frame)?;
    let width = frame.width as usize;
    let height = frame.height as usize;

    let row_bytes = width
        .checked_mul(3)
        .ok_or_else(|| WinctlError::InvalidInput("BMP row byte width overflow".to_string()))?;
    let row_stride = (row_bytes + 3) & !3;
    let pad_len = row_stride - row_bytes;
    let pixel_bytes = row_stride
        .checked_mul(height)
        .ok_or_else(|| WinctlError::InvalidInput("BMP pixel buffer size overflow".to_string()))?;
    let file_size = 54usize
        .checked_add(pixel_bytes)
        .ok_or_else(|| WinctlError::InvalidInput("BMP file size overflow".to_string()))?;

    let file_size_u32 = u32::try_from(file_size)
        .map_err(|_| WinctlError::InvalidInput("BMP file size exceeds 4 GiB limit".to_string()))?;
    let pixel_bytes_u32 = u32::try_from(pixel_bytes)
        .map_err(|_| WinctlError::InvalidInput("BMP pixel size exceeds 4 GiB limit".to_string()))?;
    let width_i32 =
        i32::try_from(frame.width).map_err(|_| WinctlError::InvalidInput("BMP width exceeds i32::MAX".to_string()))?;
    let height_i32 = i32::try_from(frame.height)
        .map_err(|_| WinctlError::InvalidInput("BMP height exceeds i32::MAX".to_string()))?;

    let mut out = Vec::with_capacity(file_size);
    // BITMAPFILEHEADER (14 bytes)
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&file_size_u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&54u32.to_le_bytes());

    // BITMAPINFOHEADER (40 bytes)
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&width_i32.to_le_bytes());
    out.extend_from_slice(&height_i32.to_le_bytes()); // positive => bottom-up
    out.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
    out.extend_from_slice(&24u16.to_le_bytes()); // biBitCount
    out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    out.extend_from_slice(&pixel_bytes_u32.to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes()); // ~72 DPI
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());

    // Bottom-up BGR scanlines with 4-byte padding
    let pad_bytes = [0u8; 3];
    for y in (0..height).rev() {
        let row_start = y * row_bytes;
        for px in rgb[row_start..row_start + row_bytes].as_chunks::<3>().0 {
            out.push(px[2]); // B
            out.push(px[1]); // G
            out.push(px[0]); // R
        }
        if pad_len > 0 {
            out.extend_from_slice(&pad_bytes[..pad_len]);
        }
    }

    Ok(out)
}

pub fn encode_ppm_p6(frame: &RawVideoFrame) -> WinctlResult<Vec<u8>> {
    let rgb = frame_to_rgb24(frame)?;
    let header = format!("P6\n{} {}\n255\n", frame.width, frame.height);
    let mut out = Vec::with_capacity(header.len() + rgb.len());
    out.extend_from_slice(header.as_bytes());
    out.extend_from_slice(&rgb);
    Ok(out)
}

pub fn webcam_table_schema() -> TableSchema {
    TableSchema::new(
        "desktop_webcam",
        "Media Foundation Video Sources",
        vec![
            ColumnDef::new("device_index", "Index", ColumnType::Uint).with_width_hint(70),
            ColumnDef::new("friendly_name", "Device Name", ColumnType::Text).with_width_hint(200),
            ColumnDef::new("symbolic_link", "Symbolic Link", ColumnType::Text).with_width_hint(260),
            ColumnDef::new("resolution", "Resolution", ColumnType::Text).with_width_hint(110),
            ColumnDef::new("pixel_format", "Pixel Format", ColumnType::Text).with_width_hint(100),
            ColumnDef::new("available", "Available", ColumnType::Bool).with_width_hint(80),
        ],
    )
}

pub fn webcam_device_to_row(id: u64, dev: &WebcamDeviceInfo) -> Row {
    Row::new(
        id,
        vec![
            CellValue::Uint(u64::from(dev.device_index)),
            CellValue::Text(dev.friendly_name.clone()),
            CellValue::Text(dev.symbolic_link.clone()),
            CellValue::Text(dev.resolution.clone()),
            CellValue::Text(dev.pixel_format.clone()),
            CellValue::Bool(dev.available),
        ],
    )
    .with_detail_json(serde_json::to_value(dev).unwrap_or(Value::Null))
}

pub struct WebcamTableProvider {
    backend: Arc<dyn DesktopBackend>,
}

impl WebcamTableProvider {
    pub fn new(backend: Arc<dyn DesktopBackend>) -> Self {
        Self { backend }
    }
}

impl TableProvider for WebcamTableProvider {
    fn tool_id(&self) -> &'static str {
        "desktop_webcam"
    }

    fn description(&self) -> &'static str {
        "Enumerate Media Foundation video capture devices and native frame capabilities"
    }

    fn schema(&self) -> TableSchema {
        webcam_table_schema()
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn query(&self, req: &TableQuery) -> Result<TableData, TableError> {
        let devices = self
            .backend
            .list_webcam_devices()
            .map_err(|e| TableError::Provider(e.to_string()))?;
        let rows: Vec<Row> = devices
            .iter()
            .enumerate()
            .map(|(idx, d)| webcam_device_to_row(idx as u64 + 1, d))
            .collect();
        apply_query(&self.schema(), &rows, req)
    }
}

#[derive(Debug, Clone, Deserialize)]
struct WebcamCaptureInput {
    #[serde(default)]
    device_index: Option<u32>,
    output_path: String,
    #[serde(default)]
    format: Option<String>,
}

pub struct WebcamCaptureAction {
    backend: Arc<dyn DesktopBackend>,
}

impl WebcamCaptureAction {
    pub fn new(backend: Arc<dyn DesktopBackend>) -> Self {
        Self { backend }
    }
}

impl ActionCommand for WebcamCaptureAction {
    fn command_id(&self) -> &'static str {
        "desktop_webcam_capture"
    }

    fn description(&self) -> &'static str {
        "Capture a still frame from a Media Foundation webcam and save as 24-bit BMP or P6 PPM"
    }

    fn risk_level(&self) -> ActionRiskLevel {
        ActionRiskLevel::Mutating
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "device_index": {
                    "type": "integer",
                    "minimum": 0,
                    "default": 0,
                    "description": "Zero-based Media Foundation video capture device index"
                },
                "output_path": {
                    "type": "string",
                    "description": "Destination image file path (.bmp or .ppm)"
                },
                "format": {
                    "type": "string",
                    "enum": ["bmp", "ppm"],
                    "description": "Output image encoding (defaults from output_path extension or 'bmp')"
                }
            },
            "required": ["output_path"]
        })
    }

    fn execute(&self, args: &Value, dry_run: bool) -> Result<Value, McpToolError> {
        let input: WebcamCaptureInput = serde_json::from_value(args.clone())
            .map_err(|e| McpToolError::InvalidParams(format!("invalid desktop_webcam_capture request: {e}")))?;

        let output_path = validate_safe_path(&input.output_path, "output_path").map_err(McpToolError::from)?;

        let mut r4_args = vec![output_path.clone()];
        if let Some(ref f) = input.format {
            r4_args.push(f.clone());
        }
        validate_r4_compliance("capture", &r4_args).map_err(McpToolError::from)?;

        let img_format = StillImageFormat::parse(input.format.as_deref(), &output_path).map_err(McpToolError::from)?;
        let device_index = input.device_index.unwrap_or(0);

        if dry_run {
            return Ok(dry_run_preview(
                self.command_id(),
                "Preview webcam capture without opening IMFSourceReader or writing output file",
                json!({
                    "device_index": device_index,
                    "output_path": output_path,
                    "format": img_format.label(),
                }),
            ));
        }

        let frame = self
            .backend
            .capture_webcam_frame(device_index)
            .map_err(McpToolError::from)?;
        let encoded = match img_format {
            StillImageFormat::Bmp => encode_bmp_24(&frame).map_err(McpToolError::from)?,
            StillImageFormat::Ppm => encode_ppm_p6(&frame).map_err(McpToolError::from)?,
        };
        let out_p = Path::new(&output_path);
        if let Some(parent) = out_p.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)
                .map_err(|e| McpToolError::ExecutionFailed(format!("failed to create directory: {e}")))?;
        }
        fs::write(out_p, &encoded)
            .map_err(|e| McpToolError::ExecutionFailed(format!("failed to write '{output_path}': {e}")))?;

        Ok(json!({
            "dry_run": false,
            "applied": true,
            "device_index": device_index,
            "output_path": output_path,
            "format": img_format.label(),
            "width": frame.width,
            "height": frame.height,
            "source_pixel_format": frame.pixel_format.label(),
            "bytes_written": encoded.len(),
            "sha256": sha256_hex(&encoded),
        }))
    }
}
