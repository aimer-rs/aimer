use aimer_cupid::canvas::CupidCanvas;
use aimer_cupid::text_pipeline::TextOverflowMode;
use aimer_cupid::utilities::Color;

/// Static international text kept visible to exercise glyph fallback and shaping.
pub(super) const WELCOME_TEXT: &str = r#"
                English — Hello / Hi               Khmer — សួស្តី (Suosdei)               French — Bonjour
                Spanish — Hola                            Portuguese — Olá                          Italian — Ciao
                German — Hallo                            Dutch — Hallo                             Swedish — Hej
                Norwegian — Hei                           Danish — Hej                              Finnish — Hei
                Icelandic — Halló                         Russian — Привет (Privet)                 Ukrainian — Привіт (Pryvit)
                Polish — Cześć                            Czech — Ahoj                              Slovak — Ahoj
                Hungarian — Szia                          Romanian — Salut                          Greek — Γεια σου (Yia sou)
                Turkish — Merhaba                         Arabic — مرحبا (Marhaban)                 Hebrew — שלום (Shalom)
                Persian — سلام (Salam)                    Hindi — नमस्ते (Namaste)                  Bengali — হ্যালো / নমস্কার
                Punjabi — ਸਤ ਸ੍ਰੀ ਅਕਾਲ                    Urdu — السلام علیکم                       Tamil — வணக்கம்
                Telugu — నమస్తే                           Kannada — ನಮಸ್ಕಾರ                         Malayalam — നമസ്കാരം
                Thai — สวัสดี                             Lao — ສະບາຍດີ                             Vietnamese — Xin chào
                Indonesian — Halo                         Malay — Hai / Halo                        Filipino — Kumusta
                Chinese (Mandarin) — 你好 (Nǐ hǎo)          Cantonese — 你好 (Néih hóu)                 Japanese — こんにちは (Konnichiwa)
                Korean — 안녕하세요 (Annyeonghaseyo)           Mongolian — Сайн байна уу                 Swahili — Jambo
                Zulu — Sawubona                           Afrikaans — Hallo                         Esperanto — Saluton
                Latin — Salve                             Hawaiian — Aloha                          Māori — Kia ora
                Extended Latin — ÀÉÎÕÜ ß Æ Œ Ł Đ Þ Ǆ Ȝ ẞ      Greek — Ελληνικά: Καλημέρα κόσμε
                Cyrillic — Русский: Добрый день мир           Armenian — Հայերեն: Բարեւ աշխարհ
                Georgian — ქართული: გამარჯობა                 Ethiopic — ሰላም ዓለም
                Hebrew — עִבְרִית: שלום עולם                  Arabic — العَرَبِيَّة: مَرْحَبًا بِالعَالَم
                Persian — فارسی: سلام دنیا                     Urdu — اردو: السلام علیکم
                Devanagari — हिन्दी: नमस्ते दुनिया             Bengali — বাংলা: শুভ সকাল
                Gurmukhi — ਪੰਜਾਬੀ: ਸਤ ਸ੍ਰੀ ਅਕਾਲ                 Gujarati — ગુજરાતી: નમસ્તે દુનિયા
                Tamil — தமிழ்: வணக்கம் உலகம்                   Telugu — తెలుగు: నమస్కారం ప్రపంచం
                Kannada — ಕನ್ನಡ: ನಮಸ್ಕಾರ ಜಗತ್ತು                 Malayalam — മലയാളം: നമസ്കാരം ലോകം
                Sinhala — සිංහල: ආයුබෝවන් ලෝකය                 Thai — ไทย: สวัสดีชาวโลก
                Lao — ລາວ: ສະບາຍດີໂລກ                        Khmer — ខ្មែរ: សួស្តី\u{200B}ពិភពលោក
                Myanmar — မြန်မာ: မင်္ဂလာပါ ကမ္ဘာ               Tibetan — བོད་ཡིག: བཀྲ་ཤིས
                CJK — 中文: 你好世界　日本語: こんにちは世界　한국어: 안녕하세요 세계
                CJK punctuation — 「」『』【】（）［］〈〉《》、。，？！：；…・—〜￥
                Combining — é å ö ñ Ž Ā क् + ष् + त्र      Direction — LTR abc / RTL אבג / مرحبا
                Symbols — © ® ™ § ¶ † ‡ № ℗ ℃ ℉ → ← ↔ ⇧ ∞ ≈ ≠ ≤ ≥ √ ∑ ∆
                Emoji — 😀 😃 🥳 🚀 ❤️ ♥️ ☕️ ✈️ 👨‍👩‍👧‍👦 🏳️‍🌈 👍🏽 🇰🇭 🇯🇵 🇺🇸
                Variation selectors — ☎︎ ☎️ ✈︎ ✈️ ☕︎ ☕️       ZWJ — 👩‍💻 🧑‍🎨 🏃‍♂️
                Private-use probes — \u{E000} \u{F8FF} \u{F0000} (unsupported glyphs stay bounded)
                                    "#;

/// Focused shaping probes for Southeast Asian scripts and combining marks.
pub(super) const SOUTHEAST_ASIAN_TEXT: &str = r#"Thai      — ไทย: สวัสดีชาวโลก | เกาะ | กำลัง
Lao       — ລາວ: ສະບາຍດີໂລກ | ເກົາ | ກຳລັງ
Khmer     — ខ្មែរ: សួស្តីពិភពលោក | ស្តី | ក្រ
Myanmar   — မြန်မာ: မင်္ဂလာပါ ကမ္ဘာ | မေ | က္က
Combining — กั ก่ ก้ | ກິ ກ່ | កា ស៊ | ကိ ကု
Mixed     — ไทย / ລາວ / ខ្មែរ / မြန်မာ / Latin"#;

pub(super) const SOUTHEAST_ASIAN_FONT_SIZE: f32 = 32.0;

pub(super) const FONT_WEIGHT_SAMPLES: &[(u16, &str)] = &[
    (200, "200 Aa"),
    (300, "300 Aa"),
    (400, "400 Aa"),
    (500, "500 Aa"),
    (600, "600 Aa"),
    (700, "700 Aa"),
    (800, "800 Aa"),
];

pub(super) const COLOR_GLYPH_SHOWCASE: &str = "😀 🥳 🚀 ❤️ 🏳️‍🌈 👍🏽";

#[cfg(feature = "wgpu")]
pub(super) const WARM_FONT_SIZES: [f32; 3] = [20.0, 32.0, 44.0];

/// Records the shared multilingual text and shaping showcase into a canvas.
pub(super) fn record_demo_frame(canvas: &CupidCanvas, width: f32, height: f32) {
    canvas.begin_frame();

    // An opaque background keeps black text readable on every surface format.
    let width = width.max(1.0);
    let height = height.max(1.0);
    canvas.fill_rect(0.0, 0.0, width, height, Color::white(), [0.0; 4]);

    // Keep the layout and clip within the drawable, including narrow windows.
    let text_width = (width - 60.0).max(1.0);
    let text_height = (height - 60.0).max(1.0);
    canvas.set_clip(30.0, 30.0, text_width, text_height);

    canvas.draw_text(
        30.0,
        30.0,
        "Aimer — Southeast Asian shaping",
        22.0,
        Color::black(),
        400,
    );
    let sea_y = 62.0;
    let sea_height = (height * 0.40).min(250.0).max(1.0);
    canvas.draw_text_with_overflow(
        30.0,
        sea_y,
        SOUTHEAST_ASIAN_TEXT,
        SOUTHEAST_ASIAN_FONT_SIZE,
        Color::black(),
        text_width,
        sea_height,
        TextOverflowMode::Wrap,
        400,
    );

    let diagnostics_y = (sea_y + sea_height + 12.0).min(height - 1.0);
    canvas.draw_text(
        30.0,
        diagnostics_y,
        "Font weights",
        18.0,
        Color::black(),
        400,
    );
    for (index, (weight, sample)) in FONT_WEIGHT_SAMPLES.iter().enumerate() {
        let column = index % 4;
        let row = index / 4;
        canvas.draw_text(
            30.0 + column as f32 * 86.0,
            diagnostics_y + 24.0 + row as f32 * 24.0,
            sample,
            18.0,
            Color::black(),
            *weight,
        );
    }

    let color_x = (width * 0.56).max(360.0).min(width - 180.0);
    canvas.draw_text(
        color_x,
        diagnostics_y,
        "Color glyphs / text",
        18.0,
        Color::black(),
        400,
    );
    canvas.draw_text(
        color_x,
        diagnostics_y + 24.0,
        "Red",
        17.0,
        Color::red(),
        400,
    );
    canvas.draw_text(
        color_x + 42.0,
        diagnostics_y + 24.0,
        "Green",
        17.0,
        Color::green(),
        400,
    );
    canvas.draw_text(
        color_x + 100.0,
        diagnostics_y + 24.0,
        "Blue",
        17.0,
        Color::blue(),
        400,
    );
    canvas.draw_text(
        color_x,
        diagnostics_y + 50.0,
        COLOR_GLYPH_SHOWCASE,
        22.0,
        Color::black(),
        400,
    );
    canvas.draw_text(
        color_x,
        diagnostics_y + 77.0,
        "COLR/CPAL + bitmap color",
        14.0,
        Color::new(0.25, 0.25, 0.25, 1.0),
        400,
    );

    let diagnostics_height = 100.0;
    let showcase_y = (diagnostics_y + diagnostics_height + 12.0).min(height - 1.0);
    let showcase_height = (height - showcase_y - 30.0).max(1.0);
    canvas.draw_text_with_overflow(
        30.0,
        showcase_y,
        WELCOME_TEXT,
        18.0,
        Color::black(),
        text_width,
        showcase_height,
        TextOverflowMode::Wrap,
        400,
    );
    canvas.clear_clip();
}
