//! Direkter USB-Zugriff über librtlsdr (rtl-sdr-blog-Fork mit V4-Unterstützung).
//!
//! Die Bibliothek wird zur Laufzeit geladen, damit das Programm auch ohne sie
//! startet (z. B. mit rtl_tcp oder Simulator) und eine klare Fehlermeldung zeigt.

use std::ffi::{c_char, c_int, c_void, CStr};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use crossbeam_channel::{Receiver, Sender};
use libloading::Library;

use super::{
    convert_u8, nearest_gain, push_block, set_status, IqBlock, SharedStatus, SourceCmd,
    BLOCK_SAMPLES, DISCARD_AFTER_RETUNE,
};
use crate::state::DeviceSettings;

type Dev = *mut c_void;

struct Api {
    _lib: Library,
    get_device_count: unsafe extern "C" fn() -> u32,
    get_device_name: unsafe extern "C" fn(u32) -> *const c_char,
    open: unsafe extern "C" fn(*mut Dev, u32) -> c_int,
    close: unsafe extern "C" fn(Dev) -> c_int,
    set_center_freq: unsafe extern "C" fn(Dev, u32) -> c_int,
    set_sample_rate: unsafe extern "C" fn(Dev, u32) -> c_int,
    set_tuner_gain_mode: unsafe extern "C" fn(Dev, c_int) -> c_int,
    set_tuner_gain: unsafe extern "C" fn(Dev, c_int) -> c_int,
    get_tuner_gains: unsafe extern "C" fn(Dev, *mut c_int) -> c_int,
    set_freq_correction: unsafe extern "C" fn(Dev, c_int) -> c_int,
    set_bias_tee: Option<unsafe extern "C" fn(Dev, c_int) -> c_int>,
    reset_buffer: unsafe extern "C" fn(Dev) -> c_int,
    read_sync: unsafe extern "C" fn(Dev, *mut c_void, c_int, *mut c_int) -> c_int,
}

impl Api {
    fn load(path: Option<&str>) -> Result<Self> {
        let candidates: Vec<&str> = match path {
            Some(p) => vec![p],
            None => vec!["librtlsdr.so.0", "librtlsdr.so", "librtlsdr.dylib", "rtlsdr.dll"],
        };
        let mut last_err = None;
        for c in candidates {
            // SAFETY: Laden einer bekannten C-Bibliothek ohne Initialisierungs-Seiteneffekte.
            match unsafe { Library::new(c) } {
                Ok(lib) => return unsafe { Self::from_lib(lib) },
                Err(e) => last_err = Some(e),
            }
        }
        Err(anyhow!("librtlsdr nicht gefunden: {}", last_err.unwrap()))
    }

    unsafe fn from_lib(lib: Library) -> Result<Self> {
        macro_rules! sym {
            ($name:literal) => {
                *lib.get(concat!($name, "\0").as_bytes())
                    .with_context(|| format!("Symbol {} fehlt in librtlsdr", $name))?
            };
        }
        let set_bias_tee = lib.get(b"rtlsdr_set_bias_tee\0").ok().map(|s| *s);
        Ok(Self {
            get_device_count: sym!("rtlsdr_get_device_count"),
            get_device_name: sym!("rtlsdr_get_device_name"),
            open: sym!("rtlsdr_open"),
            close: sym!("rtlsdr_close"),
            set_center_freq: sym!("rtlsdr_set_center_freq"),
            set_sample_rate: sym!("rtlsdr_set_sample_rate"),
            set_tuner_gain_mode: sym!("rtlsdr_set_tuner_gain_mode"),
            set_tuner_gain: sym!("rtlsdr_set_tuner_gain"),
            get_tuner_gains: sym!("rtlsdr_get_tuner_gains"),
            set_freq_correction: sym!("rtlsdr_set_freq_correction"),
            set_bias_tee,
            reset_buffer: sym!("rtlsdr_reset_buffer"),
            read_sync: sym!("rtlsdr_read_sync"),
            _lib: lib,
        })
    }
}

struct Device<'a> {
    api: &'a Api,
    dev: Dev,
    gains: Vec<i32>,
}

impl Drop for Device<'_> {
    fn drop(&mut self) {
        unsafe { (self.api.close)(self.dev) };
    }
}

impl<'a> Device<'a> {
    fn open(api: &'a Api, index: u32) -> Result<Self> {
        let count = unsafe { (api.get_device_count)() };
        if count == 0 {
            return Err(anyhow!(
                "kein RTL-SDR gefunden (eingesteckt? unter Windows per usbipd an WSL weitergereicht?)"
            ));
        }
        if index >= count {
            return Err(anyhow!("Gerät {index} existiert nicht ({count} gefunden)"));
        }
        let mut dev: Dev = std::ptr::null_mut();
        let r = unsafe { (api.open)(&mut dev, index) };
        if r < 0 || dev.is_null() {
            return Err(anyhow!(
                "Gerät konnte nicht geöffnet werden (Fehler {r}); blockiert der DVB-T-Treiber dvb_usb_rtl28xxu oder fehlen Rechte auf /dev/bus/usb?"
            ));
        }
        let n = unsafe { (api.get_tuner_gains)(dev, std::ptr::null_mut()) };
        let mut gains = vec![0; n.max(0) as usize];
        if n > 0 {
            unsafe { (api.get_tuner_gains)(dev, gains.as_mut_ptr()) };
        }
        Ok(Self { api, dev, gains })
    }

    fn apply(&mut self, cmd: &SourceCmd) {
        let a = self.api;
        unsafe {
            match *cmd {
                SourceCmd::CenterFreq(hz) => {
                    (a.set_center_freq)(self.dev, hz.min(u32::MAX as u64) as u32);
                }
                SourceCmd::SampleRate(sps) => {
                    (a.set_sample_rate)(self.dev, sps);
                    (a.reset_buffer)(self.dev);
                }
                SourceCmd::Gain(None) => {
                    (a.set_tuner_gain_mode)(self.dev, 0);
                }
                SourceCmd::Gain(Some(db)) => {
                    (a.set_tuner_gain_mode)(self.dev, 1);
                    (a.set_tuner_gain)(self.dev, nearest_gain(&self.gains, db));
                }
                SourceCmd::Ppm(ppm) => {
                    // Liefert -2, wenn der Wert unverändert ist; das ist kein Fehler.
                    (a.set_freq_correction)(self.dev, ppm);
                }
                SourceCmd::BiasTee(on) => {
                    if let Some(f) = a.set_bias_tee {
                        f(self.dev, on as c_int);
                    }
                }
            }
        }
    }
}

pub fn run(
    index: u32,
    lib: Option<String>,
    mut settings: DeviceSettings,
    cmds: Receiver<SourceCmd>,
    out: Sender<IqBlock>,
    status: SharedStatus,
) {
    let api = match Api::load(lib.as_deref()) {
        Ok(a) => a,
        Err(e) => {
            tracing::error!("{e:#}");
            set_status(&status, false, format!("{e:#}"));
            return;
        }
    };

    loop {
        // Befehle, die eintreffen, während kein Gerät offen ist, trotzdem übernehmen.
        while let Ok(cmd) = cmds.try_recv() {
            update_settings(&mut settings, &cmd);
        }
        match session(&api, index, &mut settings, &cmds, &out, &status) {
            Ok(()) => return, // DSP beendet
            Err(e) => {
                tracing::warn!("USB-Quelle: {e:#}");
                set_status(&status, false, format!("{e:#}"));
                std::thread::sleep(Duration::from_secs(2));
            }
        }
    }
}

fn update_settings(s: &mut DeviceSettings, cmd: &SourceCmd) {
    match *cmd {
        SourceCmd::CenterFreq(hz) => s.center_hz = hz,
        SourceCmd::SampleRate(r) => s.sample_rate = r,
        SourceCmd::Gain(g) => s.gain_db = g,
        SourceCmd::Ppm(p) => s.ppm = p,
        SourceCmd::BiasTee(b) => s.bias_tee = b,
    }
}

fn session(
    api: &Api,
    index: u32,
    settings: &mut DeviceSettings,
    cmds: &Receiver<SourceCmd>,
    out: &Sender<IqBlock>,
    status: &SharedStatus,
) -> Result<()> {
    let mut dev = Device::open(api, index)?;
    let name = unsafe { CStr::from_ptr((api.get_device_name)(index)) }
        .to_string_lossy()
        .into_owned();
    {
        let mut s = status.lock();
        s.device_name = name.clone();
        s.gains = dev.gains.iter().map(|g| *g as f32 / 10.0).collect();
    }
    for cmd in [
        SourceCmd::SampleRate(settings.sample_rate),
        SourceCmd::CenterFreq(settings.center_hz),
        SourceCmd::Ppm(settings.ppm),
        SourceCmd::Gain(settings.gain_db),
        SourceCmd::BiasTee(settings.bias_tee),
    ] {
        dev.apply(&cmd);
    }
    set_status(status, true, format!("verbunden: {name}"));
    tracing::info!("RTL-SDR geöffnet: {name}");

    let mut raw = vec![0u8; BLOCK_SAMPLES * 2];
    let mut discard = DISCARD_AFTER_RETUNE;
    loop {
        while let Ok(cmd) = cmds.try_recv() {
            dev.apply(&cmd);
            update_settings(settings, &cmd);
            if matches!(cmd, SourceCmd::CenterFreq(_) | SourceCmd::SampleRate(_)) {
                discard = DISCARD_AFTER_RETUNE;
            }
        }
        let mut n_read: c_int = 0;
        let r = unsafe {
            (api.read_sync)(dev.dev, raw.as_mut_ptr() as *mut c_void, raw.len() as c_int, &mut n_read)
        };
        if r < 0 {
            return Err(anyhow!("Lesefehler {r} (Stick abgezogen?)"));
        }
        if discard > 0 {
            discard -= 1;
            continue;
        }
        let mut samples = Vec::new();
        convert_u8(&raw[..n_read.max(0) as usize & !1], &mut samples);
        let block = IqBlock {
            center_hz: settings.center_hz,
            sample_rate: settings.sample_rate,
            samples,
        };
        if !push_block(out, block, status) {
            return Ok(());
        }
    }
}
