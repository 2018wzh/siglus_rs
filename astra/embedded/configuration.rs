use super::{abi, boundary, initialized, Failure, Result};
use siglus_scene_vm::{formats::gameexe, resource};
use std::{path::Path, sync::Mutex};

const fn text(s: &'static str) -> abi::Bytes {
    abi::Bytes {
        data: s.as_ptr(),
        len: s.len(),
    }
}
struct Formats([abi::Bytes; 1]);
// All spans point to immutable library-lifetime literals.
unsafe impl Sync for Formats {}
static FORMATS: Formats = Formats([text("siglus.scene_pck")]);
static PROBE_ID: Mutex<String> = Mutex::new(String::new());
pub(super) unsafe extern "C" fn describe(
    out: *mut abi::Descriptor,
    error: *mut abi::Error,
) -> abi::Status {
    boundary(error, || {
        initialized()?;
        let out = unsafe { out.as_mut() }.ok_or(abi::INVALID_ARGUMENT)?;
        *out = abi::Descriptor {
            header: abi::Header::of::<abi::Descriptor>(),
            family_id: text("siglus"),
            plugin_id: text("astra.emu.siglus"),
            version: text(env!("CARGO_PKG_VERSION")),
            capabilities: abi::CAP_CPU_SURFACE,
            formats: FORMATS.0.as_ptr(),
            format_count: 1,
            configuration: std::ptr::null(),
            configuration_count: 0,
        };
        Ok(())
    })
}
pub(super) unsafe extern "C" fn probe(
    path: abi::Bytes,
    out: *mut abi::ProbeResult,
    error: *mut abi::Error,
) -> abi::Status {
    boundary(error, || {
        initialized()?;
        let out = unsafe { out.as_mut() }.ok_or(abi::INVALID_ARGUMENT)?;
        let root = Path::new(unsafe { path.text(32768)? });
        if !root.is_dir()
            || resource::find_initial_gameexe_path(root).is_err()
            || resource::find_scene_pck_path(root).is_err()
        {
            return Err(abi::NO_MATCH.into());
        }
        let mut id = PROBE_ID.lock().map_err(|_| abi::PANICKED)?;
        *id = root
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or(abi::INVALID_ARGUMENT)?
            .to_owned();
        *out = abi::ProbeResult {
            game_id: abi::Bytes::new(id.as_bytes()),
            format: text("siglus.scene_pck"),
            confidence_permyriad: 10000,
            reserved: 0,
        };
        Ok(())
    })
}
pub(super) fn screen_size(root: &Path) -> Result<(u32, u32)> {
    let path = resource::find_initial_gameexe_path(root).map_err(Failure::native)?;
    let bytes = std::fs::read(&path).map_err(Failure::native)?;
    let text = if path
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("ini"))
    {
        String::from_utf8(bytes).map_err(Failure::native)?
    } else {
        let options = resource::load_gameexe_decode_options(root).map_err(Failure::native)?;
        gameexe::decode_gameexe_dat_bytes(&bytes, &options)
            .map_err(Failure::native)?
            .0
    };
    let config = gameexe::GameexeConfig::from_text(&text);
    let Some(entry) = config.get_entry("SCREEN_SIZE") else {
        return Ok((1280, 720));
    };
    let dimension = |index| {
        entry
            .item_unquoted(index)
            .and_then(|v| v.trim().parse::<u32>().ok())
            .filter(|n| (1..=16384).contains(n))
            .ok_or_else(|| {
                Failure::new(
                    abi::INVALID_ARGUMENT,
                    "ASTRA_EMU_SIGLUS_SCREEN_SIZE",
                    "invalid native SCREEN_SIZE",
                )
            })
    };
    Ok((dimension(0)?, dimension(1)?))
}
pub(super) fn surface(width: u32, height: u32, logical: (u32, u32)) -> Result<abi::SurfaceInfo> {
    let info = abi::SurfaceInfo {
        kind: abi::SURFACE_CPU,
        pixel_format: abi::PIXEL_RGBA8,
        color: abi::COLOR_SRGB,
        alpha: abi::ALPHA_IGNORED,
        rows: abi::ROWS_TOP_DOWN,
        backend: 0,
        logical: abi::Extent {
            width: logical.0,
            height: logical.1,
        },
        raster: abi::Extent { width, height },
        stride: u64::from(width) * 4,
    };
    info.validate()?;
    Ok(info)
}
