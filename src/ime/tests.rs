use super::*;

fn ch(c: char) -> KeyIn {
    KeyIn::new(Key::Char(c))
}

/// Type `keys` in `mode`; returns (text committed so far, preedit still showing).
fn type_in(mode: Mode, keys: &str) -> (String, String) {
    let mut ime = Ime::new();
    ime.set_mode(mode);
    let mut committed = String::new();
    for c in keys.chars() {
        let o = ime.on_key(ch(c));
        assert!(o.consumed, "{c:?} should be consumed in {mode:?}");
        committed.push_str(&o.commit);
    }
    (committed, ime.view().map(|v| v.preedit).unwrap_or_default())
}

/// Type then press Enter: the final text.
fn typed(keys: &str) -> String {
    let mut ime = Ime::new();
    ime.set_mode(Mode::Hiragana);
    let mut out = String::new();
    for c in keys.chars() {
        out.push_str(&ime.on_key(ch(c)).commit);
    }
    out.push_str(&ime.on_key(KeyIn::new(Key::Enter)).commit);
    out
}

#[test]
fn romaji_table_driven() {
    let cases: &[(&str, &str)] = &[
        // vowels and the plain rows
        ("aiueo", "あいうえお"),
        ("kakikukeko", "かきくけこ"),
        ("sasisuseso", "さしすせそ"),
        ("shishichi", "ししち"),
        ("tatituteto", "たちつてと"),
        ("tsu", "つ"),
        ("nanininuneno", "なににぬねの"),
        ("hahifuhehohu", "はひふへほふ"),
        ("mamimumemo", "まみむめも"),
        ("yayuyo", "やゆよ"),
        ("rarirurero", "らりるれろ"),
        ("wawo", "わを"),
        ("wiwe", "うぃうぇ"),
        ("wyiwye", "ゐゑ"),
        // voiced / semi-voiced
        ("gagigugego", "がぎぐげご"),
        ("zazijuzezo", "ざじじゅぜぞ"),
        ("dadidudedo", "だぢづでど"),
        ("babibubebo", "ばびぶべぼ"),
        ("papipupepo", "ぱぴぷぺぽ"),
        ("ja", "じゃ"),
        ("jijojuje", "じじょじゅじぇ"),
        // contracted sounds
        ("kyakyukyo", "きゃきゅきょ"),
        ("shashusho", "しゃしゅしょ"),
        ("sya", "しゃ"),
        ("chachucho", "ちゃちゅちょ"),
        ("cha", "ちゃ"),
        ("cya", "ちゃ"),
        ("tya", "ちゃ"),
        ("nyanyunyo", "にゃにゅにょ"),
        ("hyahyuhyo", "ひゃひゅひょ"),
        ("myamyumyo", "みゃみゅみょ"),
        ("ryaryuryo", "りゃりゅりょ"),
        ("gyagyugyo", "ぎゃぎゅぎょ"),
        ("jya", "じゃ"),
        ("zya", "じゃ"),
        ("byabyubyo", "びゃびゅびょ"),
        ("pyapyupyo", "ぴゃぴゅぴょ"),
        ("dya", "ぢゃ"),
        // loan-word sounds
        ("fafifufefo", "ふぁふぃふふぇふぉ"),
        ("vavivuvevo", "ヴァヴィヴヴェヴォ"),
        ("thi", "てぃ"),
        ("dhi", "でぃ"),
        ("dhu", "でゅ"),
        ("tha", "てゃ"),
        ("twu", "とぅ"),
        ("dwu", "どぅ"),
        ("tsa", "つぁ"),
        ("kwa", "くぁ"),
        ("gwa", "ぐぁ"),
        ("she", "しぇ"),
        ("che", "ちぇ"),
        ("ye", "いぇ"),
        ("whi", "うぃ"),
        ("qa", "くぁ"),
        ("fyu", "ふゅ"),
        // small kana
        ("xaxixuxexo", "ぁぃぅぇぉ"),
        ("laliluleloo", "ぁぃぅぇぉお"),
        ("xyaxyuxyo", "ゃゅょ"),
        ("lyalyulyo", "ゃゅょ"),
        ("xtu", "っ"),
        ("ltu", "っ"),
        ("xtsu", "っ"),
        ("xwa", "ゎ"),
        // small tsu from doubled consonants
        ("kka", "っか"),
        ("kitte", "きって"),
        ("zasshi", "ざっし"),
        ("tchi", "っち"),
        ("matcha", "まっちゃ"),
        ("ppa", "っぱ"),
        ("sshu", "っしゅ"),
        ("hakkiri", "はっきり"),
        ("bbya", "っびゃ"),
        // ん
        ("n", "ん"),
        ("nn", "ん"),
        ("nnn", "んん"),
        ("kan", "かん"),
        ("kannji", "かんじ"),
        ("nihon", "にほん"),
        ("nihonngo", "にほんご"),
        ("sanpo", "さんぽ"),
        ("kanzi", "かんじ"),
        ("konnnichiha", "こんんにちは"),
        ("konna", "こんな"),
        ("sonnna", "そんんな"),
        ("kannnaa", "かんんなあ"),
        ("n'a", "んあ"),
        ("kan'i", "かんい"),
        ("xn", "ん"),
        ("nya", "にゃ"),
        ("annya", "あんにゃ"),
        ("nnya", "んにゃ"),
        ("nihonn", "にほん"),
        ("ninja", "にんじゃ"),
        ("tenki", "てんき"),
        ("shinbun", "しんぶん"),
        // long vowel and punctuation
        ("ra-men", "らーめん"),
        ("-", "ー"),
        ("a,i.", "あ、い。"),
        ("[kaku]", "「かく」"),
        ("a/i", "あ・い"),
        ("~", "～"),
        ("a?b!", "あ？b！"),
        // digits and symbols stay as typed
        ("1", "1"),
        ("a1", "あ1"),
        ("a@", "あ@"),
        // capitals stay Latin and start a Latin run
        ("Windows", "Windows"),
        ("Aiueo", "Aiueo"),
        ("aAi", "あAi"),
        ("Hello,World", "Hello、World"),
        // words
        ("nihongo", "にほんご"),
        ("kyou", "きょう"),
        ("konnichiha", "こんにちは"),
        ("arigatou", "ありがとう"),
        ("toukyou", "とうきょう"),
        ("ohayougozaimasu", "おはようございます"),
        ("shinkansen", "しんかんせん"),
        ("gakkou", "がっこう"),
        ("zettai", "ぜったい"),
        ("denwa", "でんわ"),
        ("kanji", "かんじ"),
        ("sushi", "すし"),
        ("tsukue", "つくえ"),
        ("chikatetsu", "ちかてつ"),
        ("ryokou", "りょこう"),
        ("jyuusho", "じゅうしょ"),
        ("qi", "くぃ"),
    ];
    for (keys, want) in cases {
        assert_eq!(&typed(keys), want, "typing {keys:?}");
    }
}

#[test]
fn unfinished_romaji_is_shown_as_typed() {
    assert_eq!(type_in(Mode::Hiragana, "ky"), (String::new(), "ky".into()));
    assert_eq!(type_in(Mode::Hiragana, "kak"), (String::new(), "かk".into()));
    assert_eq!(type_in(Mode::Hiragana, "kan"), (String::new(), "かn".into()));
    assert_eq!(type_in(Mode::Hiragana, "kann"), (String::new(), "かん".into()));
    assert_eq!(type_in(Mode::Hiragana, "ts"), (String::new(), "ts".into()));
    let mut ime = Ime::new();
    ime.set_mode(Mode::Hiragana);
    for c in "kak".chars() {
        ime.on_key(ch(c));
    }
    let v = ime.view().unwrap();
    assert_eq!(v.pending, Some((1, 2)));
    assert_eq!(v.caret, Some(2));
}

#[test]
fn impossible_sequences_fall_back_to_latin() {
    assert_eq!(typed("kyk"), "kyk");
    assert_eq!(typed("qz"), "qz");
    assert_eq!(typed("xz"), "xz");
}

#[test]
fn katakana_and_half_width_modes() {
    assert_eq!(type_in(Mode::Katakana, "kata-kana"), (String::new(), "カタ\u{30fc}カナ".into()));
    assert_eq!(type_in(Mode::HalfKatakana, "kanga"), (String::new(), "ｶﾝｶﾞ".into()));
    assert_eq!(type_in(Mode::Katakana, "Abc"), (String::new(), "Abc".into()));
}

#[test]
fn full_alnum_commits_at_once() {
    let mut ime = Ime::new();
    ime.set_mode(Mode::FullAlnum);
    let mut s = String::new();
    for k in [ch('A'), ch('b'), ch('1'), KeyIn::new(Key::Space), ch('!')] {
        let o = ime.on_key(k);
        assert!(o.consumed);
        s.push_str(&o.commit);
    }
    assert_eq!(s, "Ａｂ１\u{3000}！");
    assert!(!ime.is_composing());
    // Enter and arrows still reach the app
    assert!(!ime.on_key(KeyIn::new(Key::Enter)).consumed);
}

fn press(ime: &mut Ime, k: Key) -> Out {
    ime.on_key(KeyIn::new(k))
}

fn start(keys: &str) -> Ime {
    let mut ime = Ime::new();
    ime.set_mode(Mode::Hiragana);
    for c in keys.chars() {
        ime.on_key(ch(c));
    }
    ime
}

#[test]
fn enter_commits_and_is_eaten() {
    let mut ime = start("nihon");
    let o = press(&mut ime, Key::Enter);
    assert_eq!(o, Out { commit: "にほん".into(), consumed: true });
    assert!(!ime.is_composing());
    // with nothing composing, Enter belongs to the app
    assert_eq!(press(&mut ime, Key::Enter), Out::default());
}

#[test]
fn escape_cancels_only_while_composing() {
    let mut ime = start("nihon");
    assert_eq!(press(&mut ime, Key::Esc), Out { commit: String::new(), consumed: true });
    assert!(!ime.is_composing());
    assert!(!press(&mut ime, Key::Esc).consumed);
}

#[test]
fn backspace_steps_back_through_romaji_then_kana() {
    let mut ime = start("kyo");
    assert_eq!(ime.view().unwrap().preedit, "きょ");
    press(&mut ime, Key::Backspace);
    assert!(!ime.is_composing());
    let mut ime = start("kak");
    press(&mut ime, Key::Backspace); // drops the k
    assert_eq!(ime.view().unwrap().preedit, "か");
    press(&mut ime, Key::Backspace);
    assert!(!ime.is_composing());
    assert!(!press(&mut ime, Key::Backspace).consumed);
}

#[test]
fn caret_moves_and_inserts_in_the_middle() {
    let mut ime = start("kaki");
    press(&mut ime, Key::Left);
    assert_eq!(ime.view().unwrap().caret, Some(1));
    for c in "su".chars() {
        ime.on_key(ch(c));
    }
    assert_eq!(ime.view().unwrap().preedit, "かすき");
    assert_eq!(ime.view().unwrap().caret, Some(2));
    press(&mut ime, Key::Home);
    assert_eq!(ime.view().unwrap().caret, Some(0));
    press(&mut ime, Key::Delete);
    assert_eq!(ime.view().unwrap().preedit, "すき");
    press(&mut ime, Key::End);
    assert_eq!(ime.view().unwrap().caret, Some(2));
    // moving settles a waiting n
    let mut ime = start("kan");
    press(&mut ime, Key::Left);
    assert_eq!(ime.view().unwrap().preedit, "かん");
    assert_eq!(ime.view().unwrap().caret, Some(1));
    assert_eq!(press(&mut ime, Key::Enter).commit, "かん");
}

#[test]
fn candidates_for_kana_variants() {
    let mut ime = start("kanga");
    press(&mut ime, Key::Space);
    let v = ime.view().unwrap();
    let c = v.cand.unwrap();
    assert_eq!(c.items, vec!["カンガ", "かんが", "ｶﾝｶﾞ", "ｋａｎｇａ", "kanga"]);
    assert_eq!(c.sel, 0);
    assert_eq!(v.preedit, "カンガ");
    press(&mut ime, Key::Space);
    assert_eq!(ime.view().unwrap().preedit, "かんが");
    ime.on_key(KeyIn::new(Key::Space).shift());
    assert_eq!(ime.view().unwrap().preedit, "カンガ");
    ime.on_key(KeyIn::new(Key::Space).shift()); // wraps to the last
    assert_eq!(ime.view().unwrap().preedit, "kanga");
    assert_eq!(press(&mut ime, Key::Enter), Out { commit: "kanga".into(), consumed: true });
    assert!(!ime.is_composing());
}

#[test]
fn number_keys_pick_a_candidate() {
    let mut ime = start("kana");
    press(&mut ime, Key::Space);
    let o = ime.on_key(ch('3'));
    assert_eq!(o, Out { commit: "ｶﾅ".into(), consumed: true });
    assert!(!ime.is_composing());
}

#[test]
fn typing_during_conversion_commits_the_selection_first() {
    let mut ime = start("ka");
    press(&mut ime, Key::Space);
    let o = ime.on_key(ch('k'));
    assert_eq!(o.commit, "カ");
    assert_eq!(ime.view().unwrap().preedit, "k");
}

#[test]
fn escape_and_backspace_close_the_candidate_window_only() {
    for k in [Key::Esc, Key::Backspace] {
        let mut ime = start("ka");
        press(&mut ime, Key::Space);
        assert!(ime.is_converting());
        assert!(press(&mut ime, k).consumed);
        assert!(!ime.is_converting());
        assert_eq!(ime.view().unwrap().preedit, "か");
    }
}

#[test]
fn function_keys_jump_to_a_variant() {
    let cases = [(6u8, "ひらがな"), (7, "ヒラガナ"), (8, "ﾋﾗｶﾞﾅ"), (9, "ｈｉｒａｇａｎａ"), (10, "hiragana")];
    for (n, want) in cases {
        let mut ime = start("hiragana");
        assert!(press(&mut ime, Key::F(n)).consumed);
        assert_eq!(ime.view().unwrap().preedit, want, "F{n}");
        assert_eq!(press(&mut ime, Key::Enter).commit, want);
    }
    // ctrl+k / ctrl+l are the same without the Fn layer
    let mut ime = start("ka");
    ime.on_key(ch('k').ctrl());
    assert_eq!(ime.view().unwrap().preedit, "カ");
    let mut ime = start("ka");
    ime.on_key(ch('l').ctrl());
    assert_eq!(ime.view().unwrap().preedit, "ｋａ");
    // a duplicate variant still selects its text (literal words have one form)
    let mut ime = start("Hi");
    press(&mut ime, Key::F(6));
    assert_eq!(ime.view().unwrap().preedit, "Hi");
}

#[test]
fn ctrl_u_clears_and_other_ctrl_keys_settle_then_pass() {
    let mut ime = start("nihon");
    assert!(ime.on_key(ch('u').ctrl()).consumed);
    assert!(!ime.is_composing());
    let mut ime = start("nihon");
    let o = ime.on_key(ch('c').ctrl());
    assert_eq!(o, Out { commit: "にほん".into(), consumed: false });
    assert!(!ime.is_composing());
}

#[test]
fn other_keys_settle_the_text_and_go_on() {
    for k in [Key::Up, Key::Down, Key::F(2), Key::Other] {
        let mut ime = start("ka");
        let o = press(&mut ime, k);
        assert_eq!(o, Out { commit: "か".into(), consumed: false }, "{k:?}");
    }
    // Tab settles the text but is not forwarded, so the launcher's Tab actions do not fire mid-word
    let mut ime = start("ka");
    assert_eq!(press(&mut ime, Key::Tab), Out { commit: "か".into(), consumed: true });
}

#[test]
fn toggling_commits_the_composition() {
    let mut ime = start("ka");
    assert_eq!(ime.toggle(), "か");
    assert_eq!(ime.mode(), Mode::Direct);
    assert_eq!(ime.toggle(), "");
    assert_eq!(ime.mode(), Mode::Hiragana);
    // it returns to the kana mode last used
    ime.set_mode(Mode::Katakana);
    ime.toggle();
    ime.toggle();
    assert_eq!(ime.mode(), Mode::Katakana);
    assert_eq!(ime.cycle_kana(), "");
    assert_eq!(ime.mode(), Mode::HalfKatakana);
    ime.cycle_kana();
    assert_eq!(ime.mode(), Mode::FullAlnum);
    ime.cycle_kana();
    assert_eq!(ime.mode(), Mode::Hiragana);
}

#[test]
fn wide_punctuation_option() {
    let mut ime = Ime::new();
    ime.set_mode(Mode::Hiragana);
    ime.wide_punct = true;
    for c in "a,i.".chars() {
        ime.on_key(ch(c));
    }
    assert_eq!(ime.view().unwrap().preedit, "あ，い．");
}

#[test]
fn digits_and_symbols_are_committed_at_once_when_idle() {
    let mut ime = Ime::new();
    ime.set_mode(Mode::Hiragana);
    assert_eq!(ime.on_key(ch('5')), Out { commit: "5".into(), consumed: true });
    assert_eq!(ime.on_key(ch('@')), Out { commit: "@".into(), consumed: true });
    assert!(!ime.is_composing());
}

// ---- key conflicts: what the engine may take from the rest of the launcher ----------------

fn all_keys() -> Vec<Key> {
    let mut v = vec![Key::Space, Key::Enter, Key::Backspace, Key::Delete, Key::Esc, Key::Tab, Key::Left, Key::Right, Key::Up, Key::Down, Key::Home, Key::End, Key::Other];
    v.extend((1..=12).map(Key::F));
    v
}

fn modifier_sets() -> [(bool, bool, bool); 4] {
    [(false, false, false), (true, false, false), (false, true, false), (false, false, true)]
}

#[test]
fn direct_mode_never_takes_a_key() {
    let mut ime = Ime::new();
    let mut keys = all_keys();
    keys.extend("azAZ09,.-[]/ ~!".chars().map(Key::Char));
    for k in keys {
        for (ctrl, shift, alt) in modifier_sets() {
            let o = ime.on_key(KeyIn { key: k, ctrl, shift, alt });
            assert_eq!(o, Out::default(), "{k:?} in direct mode");
        }
    }
}

#[test]
fn idle_kana_mode_takes_only_plain_typed_characters() {
    let mut ime = Ime::new();
    ime.set_mode(Mode::Hiragana);
    for k in all_keys() {
        for (ctrl, shift, alt) in modifier_sets() {
            let o = ime.on_key(KeyIn { key: k, ctrl, shift, alt });
            assert_eq!(o, Out::default(), "{k:?} ctrl={ctrl} shift={shift} alt={alt} while idle");
        }
    }
    // chords with Ctrl or Alt are never text
    for c in "azqtw ".chars() {
        assert!(!ime.on_key(ch(c).ctrl()).consumed);
        assert!(!ime.on_key(ch(c).alt()).consumed);
    }
    assert!(!ime.is_composing());
}

#[test]
fn random_key_soup_never_panics_and_keeps_the_caret_in_range() {
    // A small deterministic generator keeps this test dependency-free.
    let mut x: u32 = 0x1234_5678;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        x
    };
    let mut pool: Vec<Key> = all_keys();
    pool.extend("abcdefghijklmnopqrstuvwxyzABC0123456789-,.'[]/~?! ".chars().map(Key::Char));
    let mut ime = Ime::new();
    ime.set_mode(Mode::Hiragana);
    for _ in 0..20000 {
        let k = pool[next() as usize % pool.len()];
        let ki = KeyIn { key: k, ctrl: next() % 9 == 0, shift: next() % 7 == 0, alt: next() % 29 == 0 };
        let _ = ime.on_key(ki);
        if next() % 500 == 0 {
            ime.cycle_kana();
        }
        if let Some(v) = ime.view() {
            let n = v.preedit.chars().count();
            if let Some(c) = v.caret {
                assert!(c <= n);
            }
            if let Some((a, b)) = v.pending {
                assert!(a < b && b <= n);
            }
        }
    }
}

#[test]
fn table_is_sorted_unique_and_kana_only() {
    let t = romaji::all_entries();
    for w in t.windows(2) {
        assert!(w[0].0 < w[1].0, "{} !< {}", w[0].0, w[1].0);
    }
    for (k, v) in t {
        assert!(k.chars().all(|c| c.is_ascii_lowercase() || c == '\'' || c == '-'), "{k}");
        assert!(!v.is_empty());
    }
}

// ---- fonts: every character the engine can produce must have a glyph -------------------------

#[test]
fn every_committable_character_has_a_shinonome_glyph() {
    let font = crate::bdf::BdfFont::load("assets/fonts/shnmk12.bdf").expect("Shinonome 12x12 (run from the crate root)");
    let mut chars: std::collections::BTreeSet<char> = Default::default();
    for (_, v) in romaji::all_entries() {
        for c in v.chars() {
            chars.insert(c);
            chars.extend(kana::hira_to_kata(&c.to_string()).chars());
            chars.extend(kana::to_half_kata(&c.to_string()).chars());
        }
    }
    for c in "、。「」・ー～？！，．”’".chars() {
        chars.insert(c);
        chars.extend(kana::hira_to_kata(&c.to_string()).chars());
        chars.extend(kana::to_half_kata(&c.to_string()).chars());
    }
    chars.extend(kana::to_full_alnum(&(' '..='~').collect::<String>()).chars());
    let mut missing = Vec::new();
    for c in chars {
        // ASCII is drawn from the Latin face, not this font.
        if c.is_ascii() {
            continue;
        }
        let ok = crate::jis::unicode_to_jis(c).and_then(|j| font.glyph_jis(j)).is_some();
        if !ok {
            missing.push(c);
        }
    }
    assert!(missing.is_empty(), "no glyph in Shinonome 12x12 for: {missing:?}");
}

#[test]
fn spot_check_the_characters_the_plan_names() {
    let font = crate::bdf::BdfFont::load("assets/fonts/shnmk12.bdf").unwrap();
    for c in "あいうえおアイウエオー・、。「」～ヴｱｶﾞ･".chars() {
        let j = crate::jis::unicode_to_jis(c).unwrap_or_else(|| panic!("{c} has no JIS X 0208 mapping"));
        assert!(font.glyph_jis(j).is_some(), "{c}");
    }
}
