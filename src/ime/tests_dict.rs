//! Dictionary, converter, learning and the engine's conversion keys, on a tiny hand-written fixture
//! (original data, not taken from any dictionary).

use super::dict::{build_from_mozc_files, BuildOpts, Builder, Dict};
use super::*;
use std::path::PathBuf;

/// reading TAB left TAB right TAB cost TAB surface (the Mozc text layout)
const FIXTURE: &str = "\
かんじ\t1\t1\t3000\t漢字
かんじ\t1\t1\t3500\t感じ
かんじ\t1\t1\t4000\t幹事
かんじ\t1\t1\t9000\t幹事
かんじ\t1\t1\t5000\tかんじ
かんじ\t1\t1\t5200\t𠮷
へんかん\t1\t1\t3000\t変換
きょう\t1\t1\t3000\t今日
きょう\t1\t1\t3200\t京
は\t1\t1\t0\tは
は\t1\t1\t406\t葉
いい\t1\t1\t0\tいい
いい\t1\t1\t392\t良い
てんき\t1\t1\t3000\t天気
です\t1\t1\t40\tです
です\t1\t1\t100\tデス
わたし\t1\t1\t0\tわたし
わたし\t1\t1\t263\t私
がっこう\t1\t1\t3000\t学校
に\t1\t1\t0\tに
いく\t1\t1\t9\t行く
いく\t1\t1\t1\tいく
ー\t1\t1\t100\t棒
Latin\t1\t1\t100\tx
こうし\t1\t1\t20000\t高し
りんご\t1\t1\t3000\t林檎
ら\t1\t1\t3000\t等
わ\t1\t1\t3000\t輪
";

fn dict() -> Dict {
    let mut b = Builder::new(BuildOpts::default());
    b.add_mozc_text(FIXTURE);
    Dict::from_bytes(b.finish()).expect("fixture dictionary")
}

fn tmpdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("harbour-ime-test-{}-{}", std::process::id(), name));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn ime_with_dict() -> Ime {
    let mut ime = Ime::new();
    ime.set_dict(Some(dict()));
    ime.set_mode(Mode::Hiragana);
    ime
}

fn keys(ime: &mut Ime, s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        out.push_str(&ime.on_key(KeyIn::new(Key::Char(c))).commit);
    }
    out
}

fn press(ime: &mut Ime, k: Key) -> String {
    ime.on_key(KeyIn::new(k)).commit
}

// ---- format ----------------------------------------------------------------------------------

#[test]
fn lookup_is_cheapest_first_deduplicated_and_filtered() {
    let d = dict();
    let k = d.lookup("かんじ");
    let got: Vec<&str> = k.iter().map(|c| c.surface.as_str()).collect();
    // 幹事 appears once (the cheaper entry), 𠮷 is not in JIS X 0208 so it is gone.
    assert_eq!(got, ["漢字", "感じ", "幹事", "かんじ"]);
    assert_eq!(k[2].cost, 4000);
    assert!(d.lookup("ない").is_empty());
    assert!(d.lookup("こうし").is_empty(), "too costly");
    assert!(d.lookup("Latin").is_empty(), "a reading must be kana");
    assert!(d.lookup("ー").first().is_some(), "ー alone is a valid reading");
    assert!(d.verify());
}

#[test]
fn per_key_cap_and_displayable_filter() {
    let mut b = Builder::new(BuildOpts { max_cost: 9500, per_key: 2 });
    for (i, s) in ["一", "二", "三", "四"].iter().enumerate() {
        b.add("いち", 100 + i as u32, s);
    }
    b.add("いち", 1, "😀");
    let d = Dict::from_bytes(b.finish()).unwrap();
    let got: Vec<String> = d.lookup("いち").into_iter().map(|c| c.surface).collect();
    assert_eq!(got, ["一", "二"]);
}

#[test]
fn damaged_files_never_panic() {
    let good = {
        let mut b = Builder::new(BuildOpts::default());
        b.add_mozc_text(FIXTURE);
        b.finish()
    };
    assert!(Dict::from_bytes(Vec::new()).is_err());
    assert!(Dict::from_bytes(b"HIMEnope".to_vec()).is_err());
    assert!(Dict::from_bytes(good[..good.len() - 7].to_vec()).is_err(), "truncated");
    let mut bad_magic = good.clone();
    bad_magic[0] = b'X';
    assert!(Dict::from_bytes(bad_magic).is_err());
    let mut bad_sum = good.clone();
    let last = bad_sum.len() - 1;
    bad_sum[last] ^= 0xff;
    assert!(!Dict::from_bytes(bad_sum).unwrap().verify());
    // Flip every byte after the header in turn: lookups may differ but must not panic.
    for i in 32..good.len() {
        let mut m = good.clone();
        m[i] ^= 0xa5;
        if let Ok(d) = Dict::from_bytes(m) {
            for k in ["かんじ", "は", "きょう", "ー", "わ"] {
                let _ = d.lookup(k);
            }
        }
    }
}

#[test]
fn builds_from_files_in_ranges_and_loads_from_disk() {
    let dir = tmpdir("build");
    // Split the fixture over two files, as Mozc does.
    let (a, b) = FIXTURE.split_at(FIXTURE.len() / 2);
    let cut = b.find('\n').unwrap() + 1;
    std::fs::write(dir.join("d0.txt"), format!("{a}{}", &b[..cut])).unwrap();
    std::fs::write(dir.join("d1.txt"), &b[cut..]).unwrap();
    let out = dir.join("harbour-ime.dic");
    let (keys, bytes, _) = build_from_mozc_files(&[dir.join("d0.txt"), dir.join("d1.txt")], &BuildOpts::default(), &out).unwrap();
    assert!(keys >= 14 && bytes > 100);
    let d = Dict::open(&out).unwrap();
    assert!(d.verify());
    assert_eq!(d.len(), keys);
    // Readings from every range (あ-か, か-さ, ... ら-, ー) are found, i.e. the ranges concatenate sorted.
    for (k, s) in [("いい", "良い"), ("かんじ", "漢字"), ("きょう", "今日"), ("てんき", "天気"), ("は", "葉"), ("ら", "等"), ("わ", "輪"), ("りんご", "林檎")] {
        assert!(d.lookup(k).iter().any(|c| c.surface == s), "{k} -> {s}");
    }
    // The same bytes as building in one go.
    let mut one = Builder::new(BuildOpts::default());
    one.add_mozc_text(FIXTURE);
    assert_eq!(std::fs::read(&out).unwrap(), one.finish());
    // No input files that parse = an error, and nothing is left behind.
    std::fs::write(dir.join("junk.txt"), "not a dictionary\n").unwrap();
    assert!(build_from_mozc_files(&[dir.join("junk.txt")], &BuildOpts::default(), &dir.join("x.dic")).is_err());
    assert!(!dir.join("x.dic").exists() && !dir.join("x.tmp").exists() && !dir.join("x.pool.tmp").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mozc_line_parsing() {
    use super::dict::parse_mozc_line;
    assert_eq!(parse_mozc_line("あ\t1\t2\t30\t亜"), Some(("あ", 30, "亜")));
    assert_eq!(parse_mozc_line("あ\t1\t2\tx\t亜"), None);
    assert_eq!(parse_mozc_line("garbage"), None);
}

// ---- conversion ------------------------------------------------------------------------------

#[test]
fn segmentation_prefers_kanji_words_and_keeps_particles() {
    let conv = conv::Converter { dict: Some(dict()), ..Default::default() };
    assert_eq!(conv.segment("きょうはいいてんきです").as_deref(), Some("今日は良い天気です"));
    assert_eq!(conv.segment("わたしはがっこうにいく").as_deref(), Some("私は学校に行く"));
    // Unknown characters stay as typed.
    assert_eq!(conv.segment("てんきぜっこう").as_deref(), Some("天気ぜっこう"));
    assert_eq!(conv.segment("あ"), None, "one character is not worth splitting");
}

#[test]
fn candidates_exact_first_then_segmentation() {
    let conv = conv::Converter { dict: Some(dict()), ..Default::default() };
    assert_eq!(conv.candidates("かんじ"), ["漢字", "感じ", "幹事", "かんじ"]);
    let c = conv.candidates("かんじへんかん");
    assert_eq!(c, ["漢字変換"]);
    assert!(conv::Converter::default().candidates("かんじ").is_empty(), "no dictionary, no conversions");
}

#[test]
fn learning_reorders_and_persists() {
    let dir = tmpdir("learn");
    let path = dir.join("learn");
    let mut ime = ime_with_dict();
    ime.conv.learning = true;
    ime.load_learn(&path);
    keys(&mut ime, "kanji");
    press(&mut ime, Key::Space); // 漢字
    press(&mut ime, Key::Space); // 感じ
    press(&mut ime, Key::Space); // 幹事
    assert_eq!(press(&mut ime, Key::Enter), "幹事");
    assert!(ime.learn_dirty());
    ime.flush_learn().unwrap();
    assert!(!ime.learn_dirty());
    // A new session (fresh load) now offers 幹事 first.
    let mut again = ime_with_dict();
    again.load_learn(&path);
    keys(&mut again, "kanji");
    press(&mut again, Key::Space);
    assert_eq!(again.view().unwrap().preedit, "幹事");
    // Learning off: nothing recorded.
    let mut off = ime_with_dict();
    off.conv.learning = false;
    keys(&mut off, "kanji");
    press(&mut off, Key::Space);
    press(&mut off, Key::Enter);
    assert!(!off.learn_dirty());
    // A damaged line is skipped, the rest is kept; unknown glyphs are dropped.
    std::fs::write(&path, "かんじ\t感じ\t3\t100\nbroken line\nかんじ\t𠮷\t9\t100\n").unwrap();
    let mut l = ime_with_dict();
    l.load_learn(&path);
    assert_eq!(l.conv.learn.len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn learning_history_is_capped() {
    let mut l = learn::Learn::new();
    for i in 0..(learn::MAX_ENTRIES + 500) {
        l.record(&format!("よみ{i}"), "字", i as u64);
    }
    assert!(l.len() <= learn::MAX_ENTRIES);
    // The oldest, least used went first.
    assert!(l.get("よみ0").is_empty());
    assert!(!l.get(&format!("よみ{}", learn::MAX_ENTRIES + 400)).is_empty());
}

// ---- engine keys -----------------------------------------------------------------------------

#[test]
fn space_converts_enter_commits() {
    let mut ime = ime_with_dict();
    keys(&mut ime, "kanji");
    assert_eq!(ime.view().unwrap().preedit, "かんじ");
    press(&mut ime, Key::Space);
    let v = ime.view().unwrap();
    assert_eq!(v.preedit, "漢字");
    assert_eq!(v.cand.as_ref().unwrap().items, ["漢字", "感じ", "幹事", "かんじ", "カンジ", "ｶﾝｼﾞ", "ｋａｎｊｉ", "kanji"]);
    press(&mut ime, Key::Space);
    assert_eq!(ime.view().unwrap().preedit, "感じ");
    assert!(ime.on_key(KeyIn::new(Key::Space).shift()).consumed);
    assert_eq!(ime.view().unwrap().preedit, "漢字");
    assert_eq!(press(&mut ime, Key::Enter), "漢字");
    assert!(!ime.is_composing());
}

#[test]
fn digit_picks_within_the_page_and_pages_follow_the_selection() {
    let mut ime = ime_with_dict();
    keys(&mut ime, "kanji");
    press(&mut ime, Key::Space);
    assert_eq!(keys(&mut ime, "3"), "幹事");
    // Eight candidates = two pages of five; after stepping onto the second page, 1 means the sixth.
    keys(&mut ime, "kanji");
    press(&mut ime, Key::Space);
    for _ in 0..5 {
        press(&mut ime, Key::Space);
    }
    assert_eq!(ime.view().unwrap().cand.unwrap().sel, 5);
    assert_eq!(keys(&mut ime, "1"), "ｶﾝｼﾞ");
    // A digit beyond the page's rows does nothing but still belongs to the IME.
    keys(&mut ime, "kanji");
    press(&mut ime, Key::Space);
    for _ in 0..5 {
        press(&mut ime, Key::Space);
    }
    let out = ime.on_key(KeyIn::new(Key::Char('4')));
    assert!(out.consumed);
}

#[test]
fn esc_cancels_conversion_then_composition() {
    let mut ime = ime_with_dict();
    keys(&mut ime, "kanji");
    press(&mut ime, Key::Space);
    assert!(ime.on_key(KeyIn::new(Key::Esc)).consumed);
    assert!(ime.is_composing() && !ime.is_converting());
    assert_eq!(ime.view().unwrap().preedit, "かんじ");
    assert!(ime.on_key(KeyIn::new(Key::Esc)).consumed);
    assert!(!ime.is_composing());
    // Backspace during conversion returns to the kana.
    keys(&mut ime, "kanji");
    press(&mut ime, Key::Space);
    press(&mut ime, Key::Backspace);
    assert_eq!(ime.view().unwrap().preedit, "かんじ");
}

#[test]
fn typing_on_commits_the_selected_candidate() {
    let mut ime = ime_with_dict();
    keys(&mut ime, "kanji");
    press(&mut ime, Key::Space);
    assert_eq!(keys(&mut ime, "h"), "漢字");
    assert_eq!(ime.view().unwrap().preedit, "h");
}

#[test]
fn whole_sentence_in_one_go() {
    let mut ime = ime_with_dict();
    keys(&mut ime, "watashihagakkouniiku");
    press(&mut ime, Key::Space);
    assert_eq!(ime.view().unwrap().preedit, "私は学校に行く");
}

#[test]
fn function_keys_still_pick_kana_variants() {
    let mut ime = ime_with_dict();
    keys(&mut ime, "kanji");
    ime.on_key(KeyIn::new(Key::F(7)));
    assert_eq!(ime.view().unwrap().preedit, "カンジ");
    ime.on_key(KeyIn::new(Key::F(6)));
    assert_eq!(ime.view().unwrap().preedit, "かんじ");
    assert!(ime.view().unwrap().cand.is_some());
}

#[test]
fn without_a_dictionary_space_is_phase_one() {
    let mut ime = Ime::new();
    ime.set_mode(Mode::Hiragana);
    keys(&mut ime, "kanji");
    press(&mut ime, Key::Space);
    assert_eq!(ime.view().unwrap().preedit, "カンジ", "katakana first, as before");
    assert_eq!(ime.view().unwrap().cand.unwrap().items, ["カンジ", "かんじ", "ｶﾝｼﾞ", "ｋａｎｊｉ", "kanji"]);
}

#[test]
fn missing_or_bad_dictionary_file_is_not_fatal() {
    assert!(Dict::open(std::path::Path::new("/nonexistent/harbour-ime.dic")).is_err());
    let dir = tmpdir("bad");
    std::fs::write(dir.join("harbour-ime.dic"), b"junk").unwrap();
    assert!(Dict::open(&dir.join("harbour-ime.dic")).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Against a real generated dictionary (not in the repository): `HARBOUR_IME_TEST_DIC=/path/harbour-ime.dic
/// cargo test --lib real_dictionary -- --ignored`. Checks a few sentences and that lookups stay fast.
#[test]
#[ignore]
fn real_dictionary_sentences() {
    let Ok(path) = std::env::var("HARBOUR_IME_TEST_DIC") else { return };
    let conv = conv::Converter { dict: Some(Dict::open(std::path::Path::new(&path)).unwrap()), ..Default::default() };
    let cases = [
        ("きょうはいいてんきです", "今日は良い天気です"),
        ("わたしはがっこうにいきます", "私は学校に行きます"),
        ("あたらしいくるまをかった", "新しい車を買った"),
        ("あしたのてんきをおしえて", "明日の天気を教えて"),
        ("とうきょうとにいく", "東京都に行く"),
    ];
    let t = std::time::Instant::now();
    let mut ok = 0;
    for (r, want) in cases {
        let got = conv.segment(r).unwrap_or_default();
        println!("{r} -> {got}");
        ok += (got == want) as usize;
    }
    println!("{ok}/{} sentences, {:?}", cases.len(), t.elapsed());
    assert!(ok >= 3);
}
