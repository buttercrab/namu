use graph::{trace, workflow, TraceValue};

#[derive(Debug, Clone)]
struct Audio {
    data: Vec<f32>,
    sample_rate: u32,
}

#[trace]
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

#[trace]
fn remove_bgm(audio: Audio) -> Vec<Audio> {
    vec![audio]
}

#[trace]
fn detect_vad(audio: Audio) -> Vec<Audio> {
    vec![audio]
}

#[trace]
fn detect_language(audio: Audio) -> String {
    "en".to_string()
}

#[trace]
fn translate(audio: Audio, language: String) -> String {
    "blah blah".to_string()
}

#[trace]
fn audio_length(audio: Audio) -> f32 {
    1.0
}

#[trace]
fn cut_audio(audio: Audio, transcipt: String) -> (Audio, Audio) {
    (audio.clone(), audio)
}

#[trace]
fn save_audio(audio: Audio) {
    println!("Saving audio: {:?}", audio);
}

#[trace]
fn compare_language(language: String) -> bool {
    language == "en"
}

#[workflow]
fn workflow() -> TraceValue<()> {
    let audio = load_audios();
    let audio = remove_bgm(audio);
    let audio = detect_vad(audio);

    if !compare_language(detect_language(audio)) {
        return ();
    }

    while audio_length(audio) > 10.0 {
        let transcipt = translate(audio, "en");
        let (audio_to_save, audio_to_cut) = cut_audio(audio, transcipt);
        audio = audio_to_cut;
        save_audio(audio_to_save);
    }

    ()
}

fn main() {}
