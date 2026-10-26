//! `cargo xtask bench`: the large-workbook benchmarks (docs/large-workbook-performance.md), one
//! process at a time, with what makes two runs comparable recorded beside their results.
//!
//! Writes `target/perf/<label>/`: `commit.txt`, `working-tree.txt`, `rustc.txt`, `hardware.txt`,
//! `executable-hash.txt` (and `input-hash.txt`), then per dataset its output (`<D>.txt`, with the
//! example's p50/p95/p99), `<D>.stderr.txt` and `<D>.memory.txt` (peak memory of the process).

use std::fmt::Write as _;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::Duration;

use sha2::Digest as _;

const DATASETS: [&str; 8] = ["A", "B", "C", "F", "G", "H", "D", "E"];
const SAMPLE: Duration = Duration::from_millis(100);

#[derive(Debug, PartialEq)]
struct Options {
    label: String,
    repeats: u32,
    datasets: Vec<String>,
    /// A workbook to benchmark with `workbook_file_perf` instead of the generated datasets.
    input: Option<PathBuf>,
    /// An already built benchmark (another checkout's, for a before/after pair); skips the build.
    exe: Option<PathBuf>,
}

fn parse(args: &[&str]) -> Result<Options, String> {
    let mut o = Options { label: "current".into(), repeats: 5, datasets: DATASETS.map(String::from).to_vec(), input: None, exe: None };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = || it.next().copied().ok_or_else(|| format!("bench: `{arg}` needs a value"));
        match *arg {
            "--label" => o.label = value()?.to_string(),
            "--repeats" => o.repeats = value()?.parse().map_err(|_| "bench: --repeats takes a number".to_string())?,
            "--datasets" => o.datasets = value()?.split(',').map(|d| d.trim().to_ascii_uppercase()).filter(|d| !d.is_empty()).collect(),
            "--input" => o.input = Some(PathBuf::from(value()?)),
            "--exe" => o.exe = Some(PathBuf::from(value()?)),
            other => return Err(format!("bench: unknown option `{other}`")),
        }
    }
    // The label names a directory under target/perf: no separators or `..`.
    if o.label.is_empty() || !o.label.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')) || o.label.contains("..") {
        return Err("bench: --label takes letters, digits, `-`, `_` and `.`".into());
    }
    if !(1..=100).contains(&o.repeats) {
        return Err("bench: --repeats must be 1..=100".into());
    }
    if let Some(bad) = o.datasets.iter().find(|d| !DATASETS.contains(&d.as_str())) {
        return Err(format!("bench: unknown dataset `{bad}` (A..H)"));
    }
    if o.datasets.is_empty() {
        return Err("bench: --datasets is empty".into());
    }
    if o.input.is_some() {
        o.datasets = vec!["USER".into()];
    }
    Ok(o)
}

/// Rows the `large_workbooks` example generates for a dataset.
fn rows(dataset: &str) -> u32 {
    match dataset {
        "D" => 500_000,
        "E" => 1_000_000,
        _ => 100_000,
    }
}

/// The benchmark runs in the workspace root, so the user's `--exe` and `--input` are resolved
/// against this process's directory first: the files checked and hashed are the ones it uses.
fn resolve_paths(o: &mut Options) -> Result<(), String> {
    for path in [&mut o.exe, &mut o.input].into_iter().flatten() {
        *path = std::path::absolute(&*path).map_err(|e| format!("bench: {}: {e}", path.display()))?;
    }
    Ok(())
}

pub fn run(root: &Path, args: &[&str]) -> Result<(), String> {
    let mut o = parse(args)?;
    resolve_paths(&mut o)?;
    let example = if o.input.is_some() { "workbook_file_perf" } else { "large_workbooks" };
    let exe = match &o.exe {
        Some(exe) => exe.clone(),
        None => {
            let mut c = crate::cargo();
            c.args(["build", "--release", "-p", "gridcraft-ui-egui", "--example", example]);
            crate::run(c, &format!("cargo build --release -p gridcraft-ui-egui --example {example}"))?;
            let target =
                crate::metadata()?.get("target_directory").and_then(|t| t.as_str()).map(PathBuf::from).unwrap_or_else(|| root.join("target"));
            target.join("release").join("examples").join(format!("{example}{}", std::env::consts::EXE_SUFFIX))
        }
    };
    if !exe.is_file() {
        return Err(format!("bench: no benchmark executable at {}", exe.display()));
    }
    let out = root.join("target").join("perf").join(&o.label);
    std::fs::create_dir_all(&out).map_err(|e| format!("bench: {}: {e}", out.display()))?;
    let write = |name: &str, text: &str| std::fs::write(out.join(name), text).map_err(|e| format!("bench: {name}: {e}"));
    let commit = capture("git", &["rev-parse", "HEAD"], root);
    // An executable given with --exe was built elsewhere: the commit is this checkout's, not its.
    let commit = if o.exe.is_some() { format!("runner checkout (executable given with --exe): {commit}") } else { commit };
    write("commit.txt", &commit)?;
    write("working-tree.txt", &capture("git", &["status", "--short"], root))?;
    write("rustc.txt", &capture("rustc", &["-Vv"], root))?;
    write("hardware.txt", &hardware(root))?;
    write("executable-hash.txt", &format!("{}  {}\n", sha256(&exe)?, exe.display()))?;
    if let Some(input) = &o.input {
        write("input-hash.txt", &format!("{}  {}\n", sha256(input)?, input.display()))?;
    }
    for dataset in &o.datasets {
        let args: Vec<String> = match &o.input {
            Some(input) => vec![input.display().to_string(), o.repeats.to_string()],
            None => vec![dataset.clone(), rows(dataset).to_string(), o.repeats.to_string()],
        };
        let file = |name: String| std::fs::File::create(out.join(&name)).map_err(|e| format!("bench: {name}: {e}"));
        let mut child = Command::new(&exe)
            .args(&args)
            .current_dir(root)
            .stdin(Stdio::null())
            .stdout(file(format!("{dataset}.txt"))?)
            .stderr(file(format!("{dataset}.stderr.txt"))?)
            .spawn()
            .map_err(|e| format!("bench: {}: {e}", exe.display()))?;
        let (status, memory) = watch(&mut child)?;
        let line = format!(
            "exit_code={} peak_bytes={} sampled_peak_bytes={} method={} sampling_ms={}\n",
            status.code().map_or_else(|| "none".into(), |c| c.to_string()),
            memory.peak,
            memory.sampled,
            memory.method,
            SAMPLE.as_millis()
        );
        write(&format!("{dataset}.memory.txt"), &line)?;
        print!("{}{line}", std::fs::read_to_string(out.join(format!("{dataset}.txt"))).unwrap_or_default());
        if !status.success() {
            return Err(format!("bench: dataset {dataset} failed; see {}", out.join(format!("{dataset}.stderr.txt")).display()));
        }
    }
    println!("bench: results in {}", out.display());
    Ok(())
}

/// A command's trimmed stdout, or why there is none (the record stays best-effort).
fn capture(program: &str, args: &[&str], dir: &Path) -> String {
    match Command::new(program).args(args).current_dir(dir).stdin(Stdio::null()).stderr(Stdio::null()).output() {
        Ok(o) if o.status.success() => format!("{}\n", String::from_utf8_lossy(&o.stdout).trim_end()),
        Ok(o) => format!("unavailable: {program} exited with {}\n", o.status),
        Err(e) => format!("unavailable: {program}: {e}\n"),
    }
}

fn hardware(root: &Path) -> String {
    let mut s = format!("os={} arch={}\n", std::env::consts::OS, std::env::consts::ARCH);
    let logical = std::thread::available_parallelism().map_or(0, |n| n.get());
    let _ = writeln!(s, "logical_cpus={logical}");
    if cfg!(windows) {
        let script = "$c = Get-CimInstance Win32_Processor; $m = Get-CimInstance Win32_ComputerSystem; \
                      $o = Get-CimInstance Win32_OperatingSystem; \"cpu=$($c.Name) cores=$($c.NumberOfCores)\"; \
                      \"memory_bytes=$($m.TotalPhysicalMemory)\"; \"os_version=$($o.Caption) $($o.Version)\"";
        s += &capture("powershell", &["-NoProfile", "-NonInteractive", "-Command", script], root);
    } else if cfg!(target_os = "macos") {
        s += &capture("sysctl", &["machdep.cpu.brand_string", "hw.physicalcpu", "hw.memsize"], root);
        s += &capture("sw_vers", &[], root);
    } else if Path::new("/proc/cpuinfo").exists() {
        let first = |path: &str, key: &str| {
            std::fs::read_to_string(path).ok().and_then(|t| t.lines().find(|l| l.starts_with(key)).map(str::to_string)).unwrap_or_default()
        };
        let _ = writeln!(
            s,
            "{}\n{}\n{}",
            first("/proc/cpuinfo", "model name"),
            first("/proc/meminfo", "MemTotal"),
            first("/etc/os-release", "PRETTY_NAME")
        );
        s += &capture("uname", &["-r"], root);
    } else {
        s += &capture("sysctl", &["hw.model", "hw.ncpu", "hw.physmem"], root);
        s += &capture("uname", &["-sr"], root);
    }
    s
}

/// SHA-256 of a file, read through a shared handle: a workbook open in Excel can still be hashed.
fn sha256(path: &Path) -> Result<String, String> {
    let mut f = std::fs::File::open(path).map_err(|e| format!("bench: {}: {e}", path.display()))?;
    let mut hasher = sha2::Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf).map_err(|e| format!("bench: {}: {e}", path.display()))?;
        let Some(chunk) = buf.get(..n).filter(|c| !c.is_empty()) else { break };
        hasher.update(chunk);
    }
    Ok(hasher.finalize().iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    }))
}

/// Peak memory of a benchmark process, in bytes.
#[derive(Debug, Default, PartialEq)]
struct Memory {
    /// The operating system's high-water mark when it keeps one, else the largest sample.
    peak: u64,
    /// The largest resident size seen at a sample.
    sampled: u64,
    method: &'static str,
}

/// Waits for `child`, sampling its memory every `SAMPLE`.
fn watch(child: &mut Child) -> Result<(ExitStatus, Memory), String> {
    let pid = child.id();
    // Windows: one PowerShell watcher reads the process's peak working set until it exits.
    let windows = if cfg!(windows) { Some(windows_watcher(pid)) } else { None };
    let mut memory = Memory::default();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| format!("bench: waiting for the benchmark: {e}"))? {
            break status;
        }
        if windows.is_none() {
            sample(pid, &mut memory);
        }
        std::thread::sleep(SAMPLE);
    };
    if let Some(watcher) = windows {
        memory = watcher
            .and_then(|w| w.wait_with_output().ok())
            .and_then(|o| parse_windows_watcher(&String::from_utf8_lossy(&o.stdout)))
            .unwrap_or(Memory { method: "unavailable", ..Memory::default() });
    }
    Ok((status, memory))
}

fn windows_watcher(pid: u32) -> Option<Child> {
    let script = format!(
        "$p = Get-Process -Id {pid} -ErrorAction SilentlyContinue; $peak = 0; $ws = 0; \
         while ($p -and -not $p.HasExited) {{ try {{ $p.Refresh(); $peak = [Math]::Max($peak, $p.PeakWorkingSet64); \
         $ws = [Math]::Max($ws, $p.WorkingSet64) }} catch {{}}; Start-Sleep -Milliseconds {} }}; \"$peak $ws\"",
        SAMPLE.as_millis()
    );
    Command::new("powershell").args(["-NoProfile", "-NonInteractive", "-Command", &script]).stdin(Stdio::null()).stdout(Stdio::piped()).spawn().ok()
}

fn parse_windows_watcher(out: &str) -> Option<Memory> {
    let mut it = out.split_whitespace().map(str::parse::<u64>);
    let (peak, sampled) = (it.next()?.ok()?, it.next()?.ok()?);
    (peak > 0).then_some(Memory { peak, sampled, method: "peak-working-set" })
}

/// One sample on unix: Linux's `/proc` keeps a high-water mark (VmHWM); elsewhere `ps` gives the
/// current resident size only.
fn sample(pid: u32, m: &mut Memory) {
    if let Ok(status) = std::fs::read_to_string(format!("/proc/{pid}/status")) {
        let (hwm, rss) = (status_kib(&status, "VmHWM:"), status_kib(&status, "VmRSS:"));
        m.peak = m.peak.max(hwm.unwrap_or(0).saturating_mul(1024));
        m.sampled = m.sampled.max(rss.unwrap_or(0).saturating_mul(1024));
        m.method = "proc-vmhwm";
        return;
    }
    let rss = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<u64>().ok());
    if let Some(kib) = rss {
        m.sampled = m.sampled.max(kib.saturating_mul(1024));
        m.peak = m.peak.max(m.sampled);
        m.method = "ps-rss-sampled";
    }
}

/// The KiB value of a `/proc/<pid>/status` line such as `VmHWM:   123456 kB`.
fn status_kib(status: &str, key: &str) -> Option<u64> {
    status.lines().find_map(|l| l.strip_prefix(key)).and_then(|v| v.split_whitespace().next()).and_then(|v| v.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_default_and_validate() {
        let o = parse(&[]).unwrap();
        assert_eq!((o.label.as_str(), o.repeats, o.datasets.len(), o.input.is_none()), ("current", 5, 8, true));
        let o = parse(&["--label", "merged-1", "--repeats", "3", "--datasets", "a, e"]).unwrap();
        assert_eq!((o.label.as_str(), o.repeats, o.datasets), ("merged-1", 3, vec!["A".to_string(), "E".to_string()]));
        let o = parse(&["--input", "book.xlsx", "--datasets", "A"]).unwrap();
        assert_eq!(o.datasets, vec!["USER".to_string()]);
        for bad in [&["--label", "../x"][..], &["--label", "a/b"], &["--repeats", "0"], &["--datasets", "Z"], &["--repeats"], &["--frobnicate"]] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn user_paths_resolve_against_the_callers_directory() {
        let mut o = parse(&["--input", "book.xlsx", "--exe", "build/large_workbooks.exe"]).unwrap();
        resolve_paths(&mut o).unwrap();
        let cwd = std::env::current_dir().unwrap();
        for (path, end) in [(o.input.unwrap(), "book.xlsx"), (o.exe.unwrap(), "large_workbooks.exe")] {
            assert!(path.is_absolute() && path.starts_with(&cwd) && path.ends_with(end), "{}", path.display());
        }
    }

    #[test]
    fn dataset_rows_match_the_example_defaults() {
        assert_eq!([rows("A"), rows("D"), rows("E"), rows("H")], [100_000, 500_000, 1_000_000, 100_000]);
    }

    #[test]
    fn hashes_files_with_sha256() {
        let path = std::env::temp_dir().join(format!("gridcraft-xtask-bench-{}.txt", std::process::id()));
        std::fs::write(&path, b"abc").unwrap();
        let hash = sha256(&path);
        let _ = std::fs::remove_file(&path);
        assert_eq!(hash.unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn reads_memory_reports() {
        assert_eq!(status_kib("Name:\tx\nVmHWM:\t  2048 kB\nVmRSS:\t 1024 kB\n", "VmHWM:"), Some(2048));
        assert_eq!(status_kib("Name:\tx\n", "VmHWM:"), None);
        assert_eq!(parse_windows_watcher("4096 2048\r\n"), Some(Memory { peak: 4096, sampled: 2048, method: "peak-working-set" }));
        assert_eq!(parse_windows_watcher("0 0"), None);
        assert_eq!(parse_windows_watcher(""), None);
    }
}
