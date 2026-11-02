use std::path::PathBuf;

use aimer_utils::debug;

/// Returns the platform-specific path for the pipeline cache file.
fn cache_path() -> Option<PathBuf> {
    #[cfg(target_os = "android")]
    {
        Some(PathBuf::from("/data/local/tmp/aimer_pipeline_cache.bin"))
    }
    #[cfg(not(target_os = "android"))]
    {
        dirs::cache_dir().map(|d| d.join("aimer").join("pipeline_cache.bin"))
    }
}

/// Load pipeline cache data from disk, if available.
fn load_cache_data() -> Option<Vec<u8>> {
    let path = cache_path()?;
    match std::fs::read(&path) {
        Ok(data) => {
            debug!(
                "Pipeline cache loaded from {:?} ({} bytes)",
                path,
                data.len()
            );
            Some(data)
        }
        Err(_) => {
            debug!("No existing pipeline cache at {:?}", path);
            None
        }
    }
}

/// Create a wgpu PipelineCache, optionally seeded with previously saved data.
///
/// Returns `None` if the device does not support the `PIPELINE_CACHE` feature
/// (currently Vulkan only).
pub fn create_pipeline_cache(device: &wgpu::Device) -> Option<wgpu::PipelineCache> {
    if !device.features().contains(wgpu::Features::PIPELINE_CACHE) {
        debug!("Pipeline cache feature not supported on this device, skipping");
        return None;
    }

    let data = load_cache_data();

    let descriptor = wgpu::PipelineCacheDescriptor {
        label: Some("aimer pipeline cache"),
        data: data.as_deref(),
        fallback: true,
    };

    // SAFETY: If `data` is Some, it was previously returned from
    // `PipelineCache::get_data`.
    let cache = unsafe { device.create_pipeline_cache(&descriptor) };
    debug!("Pipeline cache created successfully");
    Some(cache)
}

/// Save the pipeline cache data to disk for next launch.
pub fn save_pipeline_cache(cache: &wgpu::PipelineCache) {
    let data = match cache.get_data() {
        Some(data) => data,
        None => {
            debug!("Pipeline cache get_data() returned None, skipping save");
            return;
        }
    };

    let Some(path) = cache_path() else {
        debug!("Could not determine pipeline cache path");
        return;
    };

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    match std::fs::write(&path, &data) {
        Ok(_) => debug!("Pipeline cache saved to {:?} ({} bytes)", path, data.len()),
        Err(e) => debug!("Failed to save pipeline cache: {}", e),
    }
}
