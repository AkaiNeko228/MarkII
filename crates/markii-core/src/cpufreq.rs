//! Real (turbo-aware) per-core clocks from Windows performance counters.
//! sysinfo only reports the static base clock on Windows; PDH's
//! "% Processor Performance" × "Processor Frequency" gives the effective one.

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::System::Performance::{
        PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterValue,
        PDH_FMT_COUNTERVALUE, PDH_FMT_DOUBLE,
    };

    type Handle = isize;
    const PDH_FMT_NOCAP100: u32 = 0x8000;
    type Counter = isize;

    pub struct CpuFreq {
        q: Handle,
        perf: Vec<Counter>,
        freq: Vec<Counter>,
    }

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    unsafe fn read(c: Counter) -> Option<f64> {
        let mut v: PDH_FMT_COUNTERVALUE = std::mem::zeroed();
        let mut ty = 0u32;
        if PdhGetFormattedCounterValue(c, PDH_FMT_DOUBLE | PDH_FMT_NOCAP100, &mut ty, &mut v) == 0 && v.CStatus == 0 {
            Some(v.Anonymous.doubleValue)
        } else {
            None
        }
    }

    impl CpuFreq {
        pub fn new(cores: usize) -> Option<Self> {
            unsafe {
                let mut q: Handle = std::mem::zeroed();
                if windows_sys::Win32::System::Performance::PdhOpenQueryW(std::ptr::null(), 0, &mut q) != 0 {
                    return None;
                }
                let mut perf = Vec::new();
                let mut freq = Vec::new();
                for i in 0..cores {
                    let mut a: Counter = std::mem::zeroed();
                    let mut b: Counter = std::mem::zeroed();
                    let pa = w(&format!("\\Processor Information(0,{i})\\% Processor Performance"));
                    let pb = w(&format!("\\Processor Information(0,{i})\\Processor Frequency"));
                    if PdhAddEnglishCounterW(q, pa.as_ptr(), 0, &mut a) != 0
                        || PdhAddEnglishCounterW(q, pb.as_ptr(), 0, &mut b) != 0
                    {
                        PdhCloseQuery(q);
                        return None;
                    }
                    perf.push(a);
                    freq.push(b);
                }
                PdhCollectQueryData(q);
                Some(Self { q, perf, freq })
            }
        }

        /// Effective MHz per logical core (needs two samples; first call may be empty).
        pub fn sample(&mut self) -> Vec<Option<u64>> {
            unsafe {
                PdhCollectQueryData(self.q);
                self.perf
                    .iter()
                    .zip(&self.freq)
                    .map(|(&p, &f)| match (read(p), read(f)) {
                        (Some(p), Some(f)) if f > 0.0 => Some((f * p / 100.0).round() as u64),
                        _ => None,
                    })
                    .collect()
            }
        }
    }

    impl Drop for CpuFreq {
        fn drop(&mut self) {
            unsafe { PdhCloseQuery(self.q) };
        }
    }
}

#[cfg(windows)]
pub use imp::CpuFreq;

#[cfg(not(windows))]
pub struct CpuFreq;
#[cfg(not(windows))]
impl CpuFreq {
    pub fn new(_: usize) -> Option<Self> { None }
    pub fn sample(&mut self) -> Vec<Option<u64>> { Vec::new() }
}
