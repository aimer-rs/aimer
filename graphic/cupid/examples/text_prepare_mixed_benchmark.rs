//! End-to-end cost of `TextPipelineV2::prepare` for a mixed-script screen.
//!
//! The request set deliberately crosses fallback boundaries: Latin, Greek,
//! Cyrillic, Armenian, Georgian, Hebrew, Arabic, Indic, Southeast Asian,
//! Tibetan, CJK, combining marks, punctuation, symbols, and emoji all arrive
//! in one frame. The benchmark reports first-use preparation separately from
//! the warm cache-hit path, so parser/fallback/raster work is not confused with
//! the steady-state instance rebuild.
//!
//! Run with:
//!
//! ```bash
//! cargo run -p aimer_cupid --example text_prepare_mixed_benchmark --release
//! ```

use std::hint::black_box;
use std::sync::Arc;
use std::time::{Duration, Instant};

use aimer_cupid::AntiAlias;
use aimer_cupid::backend::wgpu::WgpuBackend;
use aimer_cupid::font::{FontFamily, FontStyle, TextLanguage};
use aimer_cupid::text_layout::TextHorizontalAlign;
use aimer_cupid::text_pipeline::{
    TextDrawRequest, TextOverflowMode, TextPipelineV2, TextPreparationProfile,
};
use aimer_cupid::utilities::Rgba8;
use aimer_utils::SyncFuture;

const SURFACE_WIDTH: u32 = 1600;
const SURFACE_HEIGHT: u32 = 1600;
const LINE_HEIGHT: f32 = 44.0;
const FONT_SIZE: f32 = 28.0;
const MEASURED_ITERATIONS: usize = 20;

const MIXED_SAMPLES: &[(&str, Option<TextLanguage>)] = &[
    ("English — Hello / Hi; éclair, naïve, à, 👋", None),
    ("Greek — Ελληνικά: Καλημέρα κόσμε", None),
    ("Cyrillic — Русский: Добрый день мир", None),
    ("Armenian — Հայերեն: Բարեւ աշխարհ", None),
    ("Georgian — ქართული: გამარჯობა", None),
    ("Hebrew — עברית: שלום עולם", None),
    ("Arabic — العربية: مرحباً بالعالم", None),
    ("Persian — فارسی: سلام دنیا", None),
    ("Urdu — اردو: مرحبا دنیا", None),
    ("Devanagari — हिन्दी: नमस्ते दुनिया", None),
    ("Bengali — বাংলা: শুভ সকাল", None),
    ("Gurmukhi — ਪੰਜਾਬੀ: ਸਤ ਸ੍ਰੀ ਅਕਾਲ", None),
    ("Gujarati — ગુજરાતી: નમસ્તે દુનિયા", None),
    ("Tamil — தமிழ்: வணக்கம் உலகம்", None),
    ("Telugu — తెలుగు: నమస్కారం ప్రపంచం", None),
    ("Kannada — ಕನ್ನಡ: ನಮಸ್ಕಾರ ಜಗತ್ತು", None),
    ("Malayalam — മലയാളം: നമസ്കാരം ലോകം", None),
    ("Sinhala — සිංහල: ආයුබෝවන් ලෝකය", None),
    ("Thai — ไทย: สวัสดีชาวโลก", None),
    ("Lao — ລາວ: ສະບາຍດີໂລກ", None),
    ("Khmer — ខ្មែរ: សួស្តីពិភពលោក", None),
    ("Myanmar — မြန်မာ: မင်္ဂလာပါ ကမ္ဘာ", None),
    ("Tibetan — བོད་ཡིག: བཀྲ་ཤིས", None),
    ("Chinese — 中文: 你好世界", Some(TextLanguage::Chinese)),
    (
        "Japanese — 日本語: こんにちは世界",
        Some(TextLanguage::Japanese),
    ),
    (
        "Korean — 한국어: 안녕하세요 세계",
        Some(TextLanguage::Korean),
    ),
    (
        "CJK punctuation — 「」『』【】（）〈〉《》、。！？：；…·—〜¥",
        None,
    ),
    ("Combining — é å ö ñ Ž Ā ॐ + ़ + ि", None),
    (
        "Symbols — © ® ™ § ¶ † ‡ № °C °F → ← ↔ ⇧ ∞ ≈ ≠ ≤ ≥ √ Σ Δ",
        None,
    ),
];

fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::default();
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions::default())
        .block()
        .ok()
        .or_else(|| {
            instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    force_fallback_adapter: true,
                    ..Default::default()
                })
                .block()
                .ok()
        })?;
    adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("mixed text prepare benchmark device"),
            ..Default::default()
        })
        .block()
        .ok()
}

fn requests() -> Vec<TextDrawRequest> {
    let clip = [0.0, 0.0, SURFACE_WIDTH as f32, SURFACE_HEIGHT as f32];
    MIXED_SAMPLES
        .iter()
        .enumerate()
        .map(|(index, (text, language))| TextDrawRequest {
            x: 24.0,
            y: 20.0 + index as f32 * LINE_HEIGHT,
            text: Arc::from(*text),
            font_size: FONT_SIZE,
            color: Rgba8::from_unorm([0.05, 0.05, 0.05, 1.0]),
            bounds_width: SURFACE_WIDTH as f32 - 48.0,
            bounds_height: LINE_HEIGHT,
            overflow: TextOverflowMode::Wrap,
            horizontal_align: TextHorizontalAlign::Left,
            writing_mode: aimer_cupid::text_layout::TextWritingMode::HorizontalTb,
            line_height: Some(LINE_HEIGHT),
            shadow: None,
            draw_glyphs: true,
            font_family: FontFamily::SANS_SERIF,
            font_style: FontStyle::Normal,
            font_weight: None,
            language: *language,
            italic: false,
            clip_rect: clip,
            clip_border_radius: [0.0; 4],
            spans: Vec::new(),
        })
        .collect()
}

fn pipeline(backend: &WgpuBackend) -> TextPipelineV2<WgpuBackend> {
    TextPipelineV2::new(
        backend,
        wgpu::TextureFormat::Rgba8Unorm,
        AntiAlias::Analytic,
    )
}

fn prepare(
    pipeline: &mut TextPipelineV2<WgpuBackend>,
    backend: &WgpuBackend,
    requests: &[TextDrawRequest],
) -> TextPreparationProfile {
    let profile = pipeline.prepare_profiled(
        backend,
        SURFACE_WIDTH,
        SURFACE_HEIGHT,
        false,
        black_box(requests),
        &[],
    );
    black_box(pipeline.frame_glyph_instances());
    profile
}

fn shifted_requests(requests: &[TextDrawRequest], delta: f32) -> Vec<TextDrawRequest> {
    let mut shifted = requests.to_vec();
    if let Some(request) = shifted.first_mut() {
        request.x += delta;
    }
    shifted
}

fn print_stats(label: &str, mut samples: Vec<Duration>) {
    let first = samples[0];
    let total = samples.iter().copied().sum::<Duration>();
    let average = total / samples.len() as u32;
    let repeated_average = if samples.len() > 1 {
        (total - first) / (samples.len() - 1) as u32
    } else {
        average
    };
    samples.sort_unstable();
    let p50 = samples[samples.len() / 2];
    let p95 = samples[(samples.len() - 1) * 95 / 100];
    let worst = samples[samples.len() - 1];
    println!(
        "{label}: first={first:?}, avg={average:?}, repeat_avg={repeated_average:?}, \
         p50={p50:?}, p95={p95:?}, worst={worst:?}"
    );
}

fn average_profile(samples: &[TextPreparationProfile]) -> TextPreparationProfile {
    let mut average = TextPreparationProfile::default();
    if samples.is_empty() {
        return average;
    }
    for sample in samples {
        average.total += sample.total;
        average.request_analysis += sample.request_analysis;
        average.key_construction += sample.key_construction;
        average.fallback_resolution += sample.fallback_resolution;
        average.font_snapshot += sample.font_snapshot;
        average.shaping += sample.shaping;
        average.fallback_and_shaping += sample.fallback_and_shaping;
        average.layout += sample.layout;
        average.glyph_preparation += sample.glyph_preparation;
        average.atlas_planning += sample.atlas_planning;
        average.atlas_population += sample.atlas_population;
        average.instance_build += sample.instance_build;
        average.atlas_upload += sample.atlas_upload;
        average.instance_upload += sample.instance_upload;
        average.shaping_jobs += sample.shaping_jobs;
        average.layout_jobs += sample.layout_jobs;
        average.glyph_jobs += sample.glyph_jobs;
        average.alpha_glyphs += sample.alpha_glyphs;
        average.color_glyphs += sample.color_glyphs;
    }
    let count = samples.len() as u32;
    average.total /= count;
    average.request_analysis /= count;
    average.key_construction /= count;
    average.fallback_resolution /= count;
    average.font_snapshot /= count;
    average.shaping /= count;
    average.fallback_and_shaping /= count;
    average.layout /= count;
    average.glyph_preparation /= count;
    average.atlas_planning /= count;
    average.atlas_population /= count;
    average.instance_build /= count;
    average.atlas_upload /= count;
    average.instance_upload /= count;
    average.shaping_jobs /= samples.len();
    average.layout_jobs /= samples.len();
    average.glyph_jobs /= samples.len();
    average.alpha_glyphs /= samples.len();
    average.color_glyphs /= samples.len();
    average.cache_hit = samples.iter().all(|sample| sample.cache_hit);
    average
}

fn print_profile(label: &str, samples: &[TextPreparationProfile]) {
    let profile = average_profile(samples);
    println!(
        "{label}: cache_hit={}, total={:?}, request={:?}, keys={:?}, fallback={:?}, snapshot={:?}, shape={:?}, fallback+shape={:?}, layout={:?}, \
         glyph={:?}, atlas_plan={:?}, atlas_population={:?}, instance_build={:?}, \
         atlas_upload={:?}, instance_upload={:?}, jobs=(shape={}, layout={}, glyph={}), \
         descriptors=(alpha={}, color={})",
        profile.cache_hit,
        profile.total,
        profile.request_analysis,
        profile.key_construction,
        profile.fallback_resolution,
        profile.font_snapshot,
        profile.shaping,
        profile.fallback_and_shaping,
        profile.layout,
        profile.glyph_preparation,
        profile.atlas_planning,
        profile.atlas_population,
        profile.instance_build,
        profile.atlas_upload,
        profile.instance_upload,
        profile.shaping_jobs,
        profile.layout_jobs,
        profile.glyph_jobs,
        profile.alpha_glyphs,
        profile.color_glyphs,
    );
}

fn main() {
    let Some((device, queue)) = gpu() else {
        eprintln!("skipping: no GPU adapter available");
        return;
    };
    let backend = WgpuBackend::new(device, queue);
    let requests = requests();

    // Keep pipeline construction out of this timer: the benchmark is about
    // the complete prepare path, while GPU render-pipeline creation is shared
    // setup and would drown out the text first-use work.
    let mut cold_samples = Vec::with_capacity(MEASURED_ITERATIONS);
    let mut cold_profiles = Vec::with_capacity(MEASURED_ITERATIONS);
    for _ in 0..MEASURED_ITERATIONS {
        let mut pipeline = pipeline(&backend);
        let start = Instant::now();
        cold_profiles.push(prepare(&mut pipeline, &backend, &requests));
        cold_samples.push(start.elapsed());
    }

    let mut warm_pipeline = pipeline(&backend);
    let _ = prepare(&mut warm_pipeline, &backend, &requests);
    let mut warm_samples = Vec::with_capacity(MEASURED_ITERATIONS);
    let mut warm_profiles = Vec::with_capacity(MEASURED_ITERATIONS);
    for _ in 0..MEASURED_ITERATIONS {
        let start = Instant::now();
        warm_profiles.push(prepare(&mut warm_pipeline, &backend, &requests));
        warm_samples.push(start.elapsed());
    }

    let (alpha_instances, color_instances) = warm_pipeline.frame_glyph_instances();
    println!(
        "mixed TextPipelineV2::prepare: {} requests, {} scripts, {} measured iterations, glyph instances (alpha={}, color={})",
        requests.len(),
        MIXED_SAMPLES.len(),
        MEASURED_ITERATIONS,
        alpha_instances,
        color_instances,
    );
    print_stats("cold first-use prepare", cold_samples);
    print_stats("warm cache-hit prepare", warm_samples);
    print_profile(
        "cold repeated profile (excluding first)",
        &cold_profiles[1..],
    );
    print_profile("warm profile", &warm_profiles);

    let edit_a = shifted_requests(&requests, 1.0);
    let edit_b = shifted_requests(&requests, 2.0);
    let mut edit_pipeline = pipeline(&backend);
    let _ = prepare(&mut edit_pipeline, &backend, &requests);
    let mut edit_samples = Vec::with_capacity(MEASURED_ITERATIONS);
    let mut edit_profiles = Vec::with_capacity(MEASURED_ITERATIONS);
    for index in 0..MEASURED_ITERATIONS {
        let start = Instant::now();
        let edited = if index % 2 == 0 { &edit_a } else { &edit_b };
        edit_profiles.push(prepare(&mut edit_pipeline, &backend, edited));
        edit_samples.push(start.elapsed());
    }
    print_stats("warm alternating single-request edit", edit_samples);
    print_profile("warm alternating edit profile", &edit_profiles);
}
