use graph::{TracedValue, task, workflow};

#[derive(Debug, Clone)]
struct Audio {
    data: Vec<f32>,
    sample_rate: u32,
}

#[task]
fn load_audios() -> Vec<Audio> {
    vec![
        Audio {
            data: vec![1.0, 2.0, 3.0, 4.0, 5.0],
            sample_rate: 44100,
        },
        Audio {
            data: vec![6.0, 7.0, 8.0, 9.0, 10.0],
            sample_rate: 44100,
        },
    ]
}

#[task]
fn remove_bgm(audio: Audio) -> Vec<Audio> {
    vec![audio]
}

#[task]
fn detect_vad(audio: Audio) -> Vec<Audio> {
    vec![audio]
}

#[task]
fn detect_language(audio: Audio) -> String {
    "en".to_string()
}

#[task]
fn translate(audio: Audio) -> String {
    "blah blah".to_string()
}

#[task]
fn audio_length(audio: Audio) -> f32 {
    1.0
}

#[task]
fn cut_audio(audio: Audio, transcript: String) -> (Audio, Audio) {
    (audio.clone(), audio)
}

#[task]
fn save_audio(audio: Audio) {
    println!("Saving audio: {:?}", audio);
}

#[task]
fn is_not_english(language: String) -> bool {
    language != "en"
}

#[task]
fn is_long_audio(audio: Audio) -> bool {
    audio_length(audio) > 10.0
}

#[workflow]
fn workflow() {
    let audio = load_audios();
    let audio = remove_bgm(audio);
    let mut audio = detect_vad(audio);

    if is_not_english(detect_language(audio)) {
        return ();
    }

    while is_long_audio(audio) {
        let transcipt = translate(audio);
        let (audio_to_save, audio_to_cut) = cut_audio(audio, transcipt);
        audio = audio_to_cut;
        save_audio(audio_to_save);
    }
    ()
}
fn main() {}
