#[cfg(target_arch = "wasm32")]
pub async fn fetch_font_bytes(url: &str) -> Result<Vec<u8>, wasm_bindgen::JsValue> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;

    let window =
        web_sys::window().ok_or_else(|| wasm_bindgen::JsValue::from_str("window unavailable"))?;
    let response_value = JsFuture::from(window.fetch_with_str(url)).await?;
    let response: web_sys::Response = response_value.dyn_into()?;

    if !response.ok() {
        return Err(wasm_bindgen::JsValue::from_str(&format!(
            "font fetch failed with status {}",
            response.status()
        )));
    }

    let buffer = JsFuture::from(response.array_buffer()?).await?;
    let array = js_sys::Uint8Array::new(&buffer);
    let mut bytes = vec![0; array.length() as usize];
    array.copy_to(&mut bytes);

    Ok(bytes)
}

#[cfg(target_arch = "wasm32")]
pub async fn fetch_and_register_font_bytes(
    canvas: &crate::canvas::CupidCanvas,
    url: &str,
) -> Result<crate::text_layout::FontId, wasm_bindgen::JsValue> {
    let bytes = fetch_font_bytes(url).await?;
    canvas.register_font_bytes(bytes).ok_or_else(|| {
        wasm_bindgen::JsValue::from_str(&format!("failed to register font from {url}"))
    })
}

#[cfg(target_arch = "wasm32")]
pub async fn fetch_and_register_fonts(
    canvas: &crate::canvas::CupidCanvas,
    urls: &[&str],
) -> Result<Vec<crate::text_layout::FontId>, wasm_bindgen::JsValue> {
    let mut font_ids = Vec::with_capacity(urls.len());
    for url in urls {
        font_ids.push(fetch_and_register_font_bytes(canvas, url).await?);
    }
    Ok(font_ids)
}
