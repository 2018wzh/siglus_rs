use std::fmt;

use anyhow::{anyhow, Context, Result};
use kira::manager::{backend::DefaultBackend, AudioManager, AudioManagerSettings};
#[cfg(feature = "astra-hosted")]
use kira::manager::backend::Backend as KiraBackend;
use kira::manager::error::PlaySoundError;
use kira::sound::SoundData;
use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle};
#[cfg(not(target_arch = "wasm32"))]
use kira::sound::streaming::{StreamingSoundData, StreamingSoundHandle};
use kira::track::{TrackBuilder, TrackHandle};
use kira::tween::Tween;
use kira::Volume;

impl AudioHub {
    fn default_volumes() -> Self {
        Self {
            manager: None,
            bgm: None,
            se: None,
            pcm: None,
            koe: None,
            mov: None,
            bgm_base: 255,
            se_base: 255,
            pcm_base: 255,
            koe_base: 255,
            mov_base: 255,
            bgm_master: 255,
            se_master: 255,
            pcm_master: 255,
            koe_master: 255,
            mov_master: 255,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackKind {
    Bgm,
    Se,
    Pcm,
    Koe,
    Mov,
}

/// The concrete kira manager. Playback goes through track handles, which are
/// backend-agnostic; only construction and `play` need the concrete type.
enum ManagerRef {
    Default(AudioManager<DefaultBackend>),
    #[cfg(feature = "astra-hosted")]
    Tap(AudioManager<hosted_tap::TapBackend>),
}

impl ManagerRef {
    fn play<D: SoundData>(&mut self, data: D) -> Result<D::Handle, PlaySoundError<D::Error>> {
        match self {
            Self::Default(manager) => manager.play(data),
            #[cfg(feature = "astra-hosted")]
            Self::Tap(manager) => manager.play(data),
        }
    }
}

/// Astra hosted mode: PCM tap registry. The family adapter installs a
/// callback before the VM creates its `AudioHub`; every hub created while a
/// tap is installed mixes through `TapBackend` instead of the system device.
#[cfg(feature = "astra-hosted")]
pub mod hosted_tap {
    use std::{
        sync::{
            atomic::{AtomicBool, AtomicU32, Ordering},
            Arc, Mutex,
        },
        thread::{self, JoinHandle},
    };

    use kira::manager::backend::{Backend as KiraBackend, Renderer as KiraRenderer};

    /// Receives interleaved stereo f32 chunks from the kira mixer thread.
    /// The callback blocks on the consumer's bounded queue, which paces the
    /// mixer to real time.
    pub type TapCallback = Arc<dyn Fn(&[f32]) + Send + Sync>;
    /// Unblocks an outstanding write before the mixer worker is joined.
    pub type CancelCallback = Arc<dyn Fn() + Send + Sync>;

    static TAP: Mutex<Option<(TapCallback, CancelCallback)>> = Mutex::new(None);
    static TAP_SAMPLE_RATE: AtomicU32 = AtomicU32::new(48_000);

    pub fn install(callback: TapCallback, cancel: CancelCallback, sample_rate: u32) {
        TAP_SAMPLE_RATE.store(sample_rate, Ordering::Release);
        *TAP.lock().expect("pcm tap mutex poisoned") = Some((callback, cancel));
    }

    pub fn clear() {
        *TAP.lock().expect("pcm tap mutex poisoned") = None;
    }

    pub fn current() -> Option<(u32, TapCallback, CancelCallback)> {
        let tap = TAP
            .lock()
            .expect("pcm tap mutex poisoned")
            .as_ref()
            .cloned();
        tap.map(|(tap, cancel)| (TAP_SAMPLE_RATE.load(Ordering::Acquire), tap, cancel))
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct TapBackendSettings {
        pub sample_rate: u32,
    }

    impl Default for TapBackendSettings {
        fn default() -> Self {
            Self { sample_rate: 48_000 }
        }
    }

    #[derive(Debug)]
    pub struct TapBackendError {
        message: &'static str,
    }

    impl std::fmt::Display for TapBackendError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.message)
        }
    }

    impl std::error::Error for TapBackendError {}

    pub struct TapBackend {
        callback: Option<TapCallback>,
        cancel_write: CancelCallback,
        sample_rate: u32,
        cancel: Arc<AtomicBool>,
        worker: Option<JoinHandle<()>>,
    }

    impl KiraBackend for TapBackend {
        type Settings = TapBackendSettings;

        type Error = TapBackendError;

        fn setup(settings: Self::Settings) -> Result<(Self, u32), Self::Error> {
            let (_rate, callback, cancel_write) = current().ok_or(TapBackendError {
                message: "no PCM tap is installed",
            })?;
            let sample_rate = settings.sample_rate.clamp(8_000, 192_000);
            Ok((
                Self {
                    callback: Some(callback),
                    cancel_write,
                    sample_rate,
                    cancel: Arc::new(AtomicBool::new(false)),
                    worker: None,
                },
                sample_rate,
            ))
        }

        fn start(&mut self, renderer: KiraRenderer) -> Result<(), Self::Error> {
            let callback = self.callback.take().ok_or(TapBackendError {
                message: "tap backend started twice",
            })?;
            let cancel = Arc::clone(&self.cancel);
            let sample_rate = self.sample_rate;
            let worker = thread::Builder::new()
                .name("astra-siglus-audio".into())
                .spawn(move || run_tap_worker(renderer, callback, sample_rate, cancel))
                .map_err(|_| TapBackendError {
                    message: "audio worker could not start",
                })?;
            self.worker = Some(worker);
            Ok(())
        }
    }

    impl Drop for TapBackend {
        fn drop(&mut self) {
            self.cancel.store(true, Ordering::Release);
            (self.cancel_write)();
            if let Some(worker) = self.worker.take() {
                if worker.join().is_err() {
                    log::error!("event=astra.emu.siglus.audio.worker_panicked");
                }
            }
        }
    }

    fn run_tap_worker(
        mut renderer: KiraRenderer,
        callback: TapCallback,
        sample_rate: u32,
        cancel: Arc<AtomicBool>,
    ) {
        // One mixer chunk per push; the callback blocks on the host's bounded
        // queue, which keeps the mixer at real time without a timer.
        let chunk_frames = (sample_rate / 100).max(1) as usize;
        let mut buffer: Vec<f32> = Vec::with_capacity(chunk_frames * 2);
        loop {
            if cancel.load(Ordering::Acquire) {
                break;
            }
            buffer.clear();
            renderer.on_start_processing();
            for _ in 0..chunk_frames {
                if cancel.load(Ordering::Acquire) {
                    return;
                }
                let frame = renderer.process();
                buffer.push(frame.left);
                buffer.push(frame.right);
            }
            callback(&buffer);
        }
    }
}

/// Shared Kira backend.
///
/// Kira 0.9 uses `AudioManager::play` plus `StaticSoundData::output_destination(&track)`.
/// We keep one sub-track per Siglus category (BGM/SE/PCM).
pub struct AudioHub {
    manager: Option<ManagerRef>,
    bgm: Option<TrackHandle>,
    se: Option<TrackHandle>,
    pcm: Option<TrackHandle>,
    koe: Option<TrackHandle>,
    mov: Option<TrackHandle>,
    bgm_base: u8,
    se_base: u8,
    pcm_base: u8,
    koe_base: u8,
    mov_base: u8,
    bgm_master: u8,
    se_master: u8,
    pcm_master: u8,
    koe_master: u8,
    mov_master: u8,
}

impl fmt::Debug for AudioHub {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AudioHub")
            .field("enabled", &self.manager.is_some())
            .finish()
    }
}

impl AudioHub {
    pub fn new() -> Self {
        #[cfg(feature = "astra-hosted")]
        if let Some((sample_rate, _callback, _cancel)) = hosted_tap::current() {
            return Self::with_tap(sample_rate);
        }
        match AudioManager::<DefaultBackend>::new(AudioManagerSettings::default()) {
            Ok(mut manager) => {
                let bgm = manager.add_sub_track(TrackBuilder::default()).ok();
                let se = manager.add_sub_track(TrackBuilder::default()).ok();
                let pcm = manager.add_sub_track(TrackBuilder::default()).ok();
                let koe = manager.add_sub_track(TrackBuilder::default()).ok();
                let mov = manager.add_sub_track(TrackBuilder::default()).ok();
                Self {
                    manager: Some(ManagerRef::Default(manager)),
                    bgm,
                    se,
                    pcm,
                    koe,
                    mov,
                    ..Self::default_volumes()
                }
            }
            Err(e) => {
                eprintln!("kira init failed, audio disabled: {:#}", e);
                Self::disabled()
            }
        }
    }

    /// Astra hosted mode: mixes through `TapBackend` so a family adapter can
    /// consume the output instead of the system audio device.
    #[cfg(feature = "astra-hosted")]
    fn with_tap(sample_rate: u32) -> Self {
        match AudioManager::<hosted_tap::TapBackend>::new(AudioManagerSettings {
            backend_settings: hosted_tap::TapBackendSettings { sample_rate },
            ..Default::default()
        }) {
            Ok(mut manager) => {
                let bgm = manager.add_sub_track(TrackBuilder::default()).ok();
                let se = manager.add_sub_track(TrackBuilder::default()).ok();
                let pcm = manager.add_sub_track(TrackBuilder::default()).ok();
                let koe = manager.add_sub_track(TrackBuilder::default()).ok();
                let mov = manager.add_sub_track(TrackBuilder::default()).ok();
                Self {
                    manager: Some(ManagerRef::Tap(manager)),
                    bgm,
                    se,
                    pcm,
                    koe,
                    mov,
                    ..Self::default_volumes()
                }
            }
            Err(e) => {
                eprintln!("kira tap init failed, audio disabled: {e:#}");
                Self::disabled()
            }
        }
    }

    fn disabled() -> Self {
        Self {
            manager: None,
            bgm: None,
            se: None,
            pcm: None,
            koe: None,
            mov: None,
            ..Self::default_volumes()
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.manager.is_some()
    }

    fn track_ref(&self, kind: TrackKind) -> Option<&TrackHandle> {
        match kind {
            TrackKind::Bgm => self.bgm.as_ref(),
            TrackKind::Se => self.se.as_ref(),
            TrackKind::Pcm => self.pcm.as_ref(),
            TrackKind::Koe => self.koe.as_ref(),
            TrackKind::Mov => self.mov.as_ref(),
        }
    }

    fn track_mut(&mut self, kind: TrackKind) -> Option<&mut TrackHandle> {
        match kind {
            TrackKind::Bgm => self.bgm.as_mut(),
            TrackKind::Se => self.se.as_mut(),
            TrackKind::Pcm => self.pcm.as_mut(),
            TrackKind::Koe => self.koe.as_mut(),
            TrackKind::Mov => self.mov.as_mut(),
        }
    }

    fn track_base_mut(&mut self, kind: TrackKind) -> &mut u8 {
        match kind {
            TrackKind::Bgm => &mut self.bgm_base,
            TrackKind::Se => &mut self.se_base,
            TrackKind::Pcm => &mut self.pcm_base,
            TrackKind::Koe => &mut self.koe_base,
            TrackKind::Mov => &mut self.mov_base,
        }
    }

    fn track_master_mut(&mut self, kind: TrackKind) -> &mut u8 {
        match kind {
            TrackKind::Bgm => &mut self.bgm_master,
            TrackKind::Se => &mut self.se_master,
            TrackKind::Pcm => &mut self.pcm_master,
            TrackKind::Koe => &mut self.koe_master,
            TrackKind::Mov => &mut self.mov_master,
        }
    }

    fn track_base(&self, kind: TrackKind) -> u8 {
        match kind {
            TrackKind::Bgm => self.bgm_base,
            TrackKind::Se => self.se_base,
            TrackKind::Pcm => self.pcm_base,
            TrackKind::Koe => self.koe_base,
            TrackKind::Mov => self.mov_base,
        }
    }

    fn track_master(&self, kind: TrackKind) -> u8 {
        match kind {
            TrackKind::Bgm => self.bgm_master,
            TrackKind::Se => self.se_master,
            TrackKind::Pcm => self.pcm_master,
            TrackKind::Koe => self.koe_master,
            TrackKind::Mov => self.mov_master,
        }
    }

    fn apply_track_volume(&mut self, kind: TrackKind) {
        let base = self.track_base(kind) as f64 / 255.0;
        let master = self.track_master(kind) as f64 / 255.0;
        let amp = base * master;
        let Some(track) = self.track_mut(kind) else {
            return;
        };
        track.set_volume(Volume::Amplitude(amp), Tween::default());
    }

    fn apply_track_volume_fade(&mut self, kind: TrackKind, fade_ms: i64) {
        let base = self.track_base(kind) as f64 / 255.0;
        let master = self.track_master(kind) as f64 / 255.0;
        let amp = base * master;
        let Some(track) = self.track_mut(kind) else {
            return;
        };
        let tween = if fade_ms > 0 {
            Tween {
                duration: std::time::Duration::from_millis(fade_ms as u64),
                ..Tween::default()
            }
        } else {
            Tween::default()
        };
        track.set_volume(Volume::Amplitude(amp), tween);
    }

    pub fn play_static(
        &mut self,
        kind: TrackKind,
        data: StaticSoundData,
    ) -> Result<StaticSoundHandle> {
        // Decide output destination before taking a mutable borrow of the manager.
        let data = if let Some(track) = self.track_ref(kind) {
            data.output_destination(track)
        } else {
            data
        };

        let manager = self
            .manager
            .as_mut()
            .ok_or_else(|| anyhow!("audio disabled"))?;

        manager.play(data).context("kira: play static sound")
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn play_streaming<E: Send + 'static>(
        &mut self,
        kind: TrackKind,
        data: StreamingSoundData<E>,
    ) -> Result<StreamingSoundHandle<E>> {
        let data = if let Some(track) = self.track_ref(kind) {
            data.output_destination(track)
        } else {
            data
        };
        let manager = self
            .manager
            .as_mut()
            .ok_or_else(|| anyhow!("audio disabled"))?;
        manager
            .play(data)
            .map_err(|_| anyhow!("kira: play streaming sound failed"))
    }

    pub fn set_track_volume_raw(&mut self, kind: TrackKind, volume_raw: u8) {
        *self.track_base_mut(kind) = volume_raw;
        self.apply_track_volume(kind);
    }

    pub fn set_track_master_volume_raw(&mut self, kind: TrackKind, volume_raw: u8) {
        *self.track_master_mut(kind) = volume_raw;
        self.apply_track_volume(kind);
    }

    pub fn set_track_volume_raw_fade(&mut self, kind: TrackKind, volume_raw: u8, fade_ms: i64) {
        *self.track_base_mut(kind) = volume_raw;
        self.apply_track_volume_fade(kind, fade_ms);
    }
}

