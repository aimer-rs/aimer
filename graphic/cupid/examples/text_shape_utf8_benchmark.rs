//! What shaping a few kilobytes of text costs, by the text's script.
//!
//! The renderer shapes every new chunk of a long document as it scrolls into view, so the
//! time a chunk takes is time a frame does not have. Rust text is UTF-8: a note is Latin with
//! accents, Cyrillic, Greek, CJK, emoji, and plain ASCII in whatever mix its author wrote, and
//! none of those should be the slow case. This reports milliseconds per kilobyte for each kind,
//! with the rasterizer warm as it is mid-scroll, so a script that costs several times as much
//! as ASCII shows.
//!
//! ```bash
//! cargo run -p aimer_cupid --example text_shape_utf8_benchmark
//! ```

use std::hint::black_box;
use aimer_time::Instant;

use aimer_cupid::font::{FontFamily, FontStyle, FontWeight};
use aimer_cupid::text_layout::shape_text_styled;
use aimer_cupid::text_pipeline::glyph_rasterizer::GlyphRasterizer;

const BYTES: usize = 4096;
const RUNS: usize = 40;

/// Lines of prose of about 100 characters, one hard line each, repeated with the line number
/// so that no two are alike.
fn corpus(sentence: &str) -> String {
    let mut text = String::new();
    let mut line = 0;
    while text.len() < BYTES {
        text.push_str(&format!("{line:>3} {sentence}\n"));
        line += 1;
    }
    text
}

fn main() {
    let corpora: Vec<(&str, String)> = vec![
        ("ASCII", corpus("The quick brown fox jumps over the lazy dog and keeps running through the long grass of the field.")),
        ("Latin accents", corpus("Le cœur de l'élève était brûlé par l'été, où l'on s'est déjà promené près de la forêt noire.")),
        ("Vietnamese", corpus("Tiếng Việt có nhiều dấu thanh và dấu mũ, như ở những chữ trường, được, người, và nước.")),
        ("Cyrillic", corpus("Съешь же ещё этих мягких французских булок, да выпей чаю, пока не наступила ночь над городом.")),
        ("Greek", corpus("Ξεσκεπάζω την ψυχοφθόρα βδελυγμία, και ακούω τον ήχο της θάλασσας μέσα στη νύχτα.")),
        ("CJK", corpus("国家的发展需要每一个人的努力，我们要在学习和工作中不断进步，创造更加美好的生活。")),
        ("Japanese", corpus("吾輩は猫である。名前はまだ無い。どこで生れたかとんと見当がつかぬ。何でも薄暗いところで。")),
        ("Khmer", corpus("ភាសាខ្មែរគឺជាភាសាផ្លូវការរបស់ប្រទេសកម្ពុជា ហើយមានអក្សរសម្រាប់សរសេរ។")),
        ("Arabic", corpus("اللغة العربية من أكثر اللغات انتشارا في العالم وتكتب من اليمين إلى اليسار في كل الكتب.")),
        ("ASCII + emoji", corpus("A note with a rocket 🚀 and a check ✅ in the middle of an otherwise plain English line of text.")),
        ("ASCII + one é", {
            let mut text = corpus("A plain English line of text that goes on for a while, with nothing special in it at all.");
            text.insert(BYTES / 2, 'é');
            text
        }),
    ];
    // `-- NAME` shapes only the corpus of that name, and `-- NAME N` runs them N
    // times, for a profiler to look at.
    let mut args = std::env::args().skip(1);
    let only = args.next();
    let runs = args.next().and_then(|runs| runs.parse().ok()).unwrap_or(RUNS);
    let corpora: Vec<_> = corpora.into_iter().filter(|(name, _)| only.as_deref().is_none_or(|only| *name == only)).collect();
    println!("shaping {BYTES} bytes at a time, warm, fastest of {RUNS} runs: milliseconds per shape, and per kilobyte");
    let mut rasterizer = GlyphRasterizer::new();
    // Every corpus once to settle the rasterizer, then again for the numbers.
    for (_, text) in &corpora {
        for _ in 0..3 {
            black_box(shape_text_styled(&mut rasterizer, text, 16.0, FontFamily::SANS_SERIF, FontWeight::Normal, FontStyle::Normal, None));
        }
    }
    for (name, text) in &corpora {
        // Warm: the glyphs, advances and faces this text needs are cached, as they are once a
        // document has been scrolled a little.
        for _ in 0..3 {
            black_box(shape_text_styled(&mut rasterizer, text, 16.0, FontFamily::SANS_SERIF, FontWeight::Normal, FontStyle::Normal, None));
        }
        // The fastest of several runs: another process on the machine only ever makes one slower.
        let mut each = std::time::Duration::MAX;
        for _ in 0..runs {
            let started = Instant::now();
            black_box(shape_text_styled(&mut rasterizer, black_box(text), 16.0, FontFamily::SANS_SERIF, FontWeight::Normal, FontStyle::Normal, None));
            each = each.min(started.elapsed().into());
        }
        println!("{name:<14} {:>8} bytes  {each:>10.2?}  {:>8.3} ms/KB", text.len(), each.as_secs_f64() * 1000.0 / (text.len() as f64 / 1024.0));
    }
}
