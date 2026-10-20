//! Bounded, cancellable child-process transport. Temporary files avoid pipe deadlocks and
//! never touch workbook/ID-map files. A child is killed and reaped on every early return.
use super::*;
use std::{
    fs::File,
    io::{BufRead, BufReader, Read, Seek, Write},
    process::{Child, Command, Stdio},
    sync::atomic::Ordering,
    time::{Duration, Instant},
};
const MAX_BYTES: u64 = 64 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(1800);
#[derive(Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
enum Event {
    Progress { request_id: String, revision: String, stage: String },
    Result { request_id: String, revision: String, validator: Version, report: Report },
    Error { request_id: Option<String>, revision: Option<String>, validator: Version, message: String },
}
struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
struct Bounded<'a> {
    file: &'a mut File,
    remaining: u64,
}
impl Write for Bounded<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() as u64 > self.remaining {
            return Err(std::io::Error::other("validator request exceeds 64 MiB; save the workbook first"));
        }
        let n = self.file.write(bytes)?;
        self.remaining = self.remaining.saturating_sub(n as u64);
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}
fn io(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn identity(request: &Request, id: Option<&str>, revision: Option<&str>) -> Result<(), String> {
    if id != Some(&request.request_id) || revision != Some(&request.revision) {
        return Err("패처 검사 응답의 요청/수정 버전이 일치하지 않습니다.".into());
    }
    Ok(())
}
fn version(v: &Version) -> Result<(), String> {
    if v.protocol_version != PROTOCOL_VERSION {
        return Err(format!("지원하지 않는 ck-validator 프로토콜: {}", v.protocol_version));
    }
    Ok(())
}
#[allow(clippy::disallowed_methods)] // This module is desktop-only.
pub(super) fn check(config: &Config, request: &Request, cancel: &AtomicBool, mut progress: impl FnMut(&str)) -> Result<Checked, String> {
    config.check()?;
    let mut input = tempfile::tempfile().map_err(io)?;
    serde_json::to_writer(Bounded { file: &mut input, remaining: MAX_BYTES }, request).map_err(io)?;
    input.rewind().map_err(io)?;
    let mut errors = tempfile::tempfile().map_err(io)?;
    // A reopened descriptor has an independent read cursor (try_clone shares the offset).
    let output_path = tempfile::NamedTempFile::new().map_err(io)?;
    let output_writer = output_path.reopen().map_err(io)?;
    let mut reader = BufReader::new(output_path.reopen().map_err(io)?);
    if cancel.load(Ordering::Relaxed) {
        return Err("검사가 취소되었습니다.".into());
    }
    let mut cmd = Command::new(&config.executable);
    cmd.arg("check").stdin(Stdio::from(input)).stdout(Stdio::from(output_writer)).stderr(Stdio::from(errors.try_clone().map_err(io)?));
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let mut child = Process(cmd.spawn().map_err(|e| format!("ck-validator 실행 실패 ({}): {e}", config.executable))?);
    let started = Instant::now();
    let mut pending = Vec::new();
    let mut terminal = None;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("검사가 취소되었습니다.".into());
        }
        if started.elapsed() > TIMEOUT {
            return Err("패처 검사가 30분을 초과했습니다. 작업을 취소했습니다.".into());
        }
        if output_path.as_file().metadata().map_err(io)?.len() > MAX_BYTES || errors.metadata().map_err(io)?.len() > 1024 * 1024 {
            return Err("패처 검사 응답 크기 한도를 넘었습니다.".into());
        }
        let status = child.0.try_wait().map_err(io)?;
        loop {
            let bytes = reader.fill_buf().map_err(io)?;
            if bytes.is_empty() {
                break;
            }
            let newline = bytes.iter().position(|b| *b == b'\n');
            let n = newline.map_or(bytes.len(), |i| i + 1);
            if pending.len().saturating_add(n) as u64 > MAX_BYTES {
                return Err("패처 검사 응답이 너무 큽니다.".into());
            }
            pending.extend_from_slice(bytes.get(..n).unwrap_or_default());
            reader.consume(n);
            if newline.is_none() {
                continue;
            }
            let event: Event = serde_json::from_slice(&pending).map_err(|e| format!("ck-validator JSONL 오류: {e}"))?;
            pending.clear();
            if terminal.is_some() {
                return Err("패처가 검사 완료 뒤 추가 응답을 보냈습니다.".into());
            }
            match event {
                Event::Progress { request_id, revision, stage } => {
                    identity(request, Some(&request_id), Some(&revision))?;
                    progress(&stage);
                }
                Event::Result { request_id, revision, validator, report } => {
                    identity(request, Some(&request_id), Some(&revision))?;
                    version(&validator)?;
                    let stages = ["raw", "parse", "versions", "validation"];
                    if report.stages.len() != stages.len()
                        || report
                            .stages
                            .iter()
                            .zip(stages)
                            .any(|(s, name)| s.stage != name || !["passed", "failed", "skipped"].contains(&s.status.as_str()))
                        || (report.valid && report.stages.iter().any(|s| s.status != "passed"))
                        || (!report.valid && !report.stages.iter().any(|s| s.status == "failed"))
                    {
                        return Err("패처 검사 단계와 성공 여부가 일치하지 않습니다.".into());
                    }
                    terminal = Some(Ok(Checked { validator, report }));
                }
                Event::Error { request_id, revision, validator, message } => {
                    identity(request, request_id.as_deref(), revision.as_deref())?;
                    version(&validator)?;
                    terminal = Some(Err(message));
                }
            }
        }
        if let Some(status) = status {
            if !pending.is_empty() {
                return Err("패처 검사 응답이 중간에 끊겼습니다.".into());
            }
            errors.rewind().map_err(io)?;
            let mut stderr = String::new();
            errors.take(16 * 1024).read_to_string(&mut stderr).map_err(io)?;
            let result = terminal.ok_or_else(|| format!("ck-validator가 결과 없이 종료되었습니다 ({status}): {stderr}"))??;
            let expected = if result.report.valid { 0 } else { 1 };
            if status.code() != Some(expected) {
                return Err(format!("패처 결과와 종료 상태가 일치하지 않습니다 ({status}): {stderr}"));
            }
            return Ok(result);
        }
        std::thread::sleep(Duration::from_millis(30));
    }
}
