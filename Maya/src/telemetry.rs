use crate::config::RuntimeConfig;
use crate::db::Db;
use crate::error::Result;
use crate::types::{
    ProcessCategory, ProcessInfo, ProcessLease, ProcessLeaseState, ResourceGroup,
    ResourceMeasurementStatus, ResourceSnapshot, SystemResources,
};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const PAGE_SIZE_BYTES: u64 = 4096;
const MAX_PROCESSES_IN_SNAPSHOT: usize = 100;
const CORE_RAM_LIMIT_WARN_BYTES: u64 = 400 * 1024 * 1024; // 400 MB (towards 500 MB limit)
const LOW_AVAILABLE_RAM_WARN_BYTES: u64 = 500 * 1024 * 1024; // 500 MB
const LOW_STORAGE_WARN_BYTES: u64 = 1024 * 1024 * 1024; // 1 GiB

#[derive(Clone)]
pub struct ResourceMonitor {
    db: Db,
    config: Arc<RuntimeConfig>,
    tx: broadcast::Sender<ResourceSnapshot>,
}

impl ResourceMonitor {
    pub fn new(db: Db, config: Arc<RuntimeConfig>) -> Self {
        let (tx, _) = broadcast::channel(16);
        Self { db, config, tx }
    }

    pub fn db(&self) -> &Db {
        &self.db
    }

    pub fn config(&self) -> &RuntimeConfig {
        &self.config
    }

    pub async fn sample(&self, include_processes: bool) -> Result<ResourceSnapshot> {
        let now = chrono::Utc::now().timestamp_millis();
        let mut warnings = Vec::new();

        // 1. Core process measurement
        let core_pid = std::process::id();
        let (core_rss, core_status) = Self::read_process_rss(core_pid);

        if let Some(rss) = core_rss
            && rss > CORE_RAM_LIMIT_WARN_BYTES
        {
            warnings.push(format!(
                "Maya core RSS ({} MB) approaching 500 MB limit",
                rss / (1024 * 1024)
            ));
        }

        let core_group = ResourceGroup {
            category: ProcessCategory::Core,
            pids: vec![core_pid],
            rss_bytes: core_rss,
            cpu_percent_one_core: Some(0.0),
            measurement_status: core_status,
        };

        // 2. External processes from active leases in DB
        let leases = self
            .db
            .read(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, job_id, category, pid, start_ticks, pgid, owned, boot_id, state
                     FROM process_leases
                     WHERE state IN ('starting', 'running')",
                )?;

                let rows = stmt
                    .query_map([], |row| {
                        let id_str: String = row.get(0)?;
                        let job_id_str: Option<String> = row.get(1)?;
                        let cat_str: String = row.get(2)?;
                        let pid: Option<i64> = row.get(3)?;
                        let start_ticks: Option<i64> = row.get(4)?;
                        let pgid: Option<i64> = row.get(5)?;
                        let owned: i64 = row.get(6)?;
                        let boot_id: String = row.get(7)?;
                        let state_str: String = row.get(8)?;

                        let id = Uuid::parse_str(&id_str).unwrap_or_default();
                        let job_id = job_id_str.and_then(|s| Uuid::parse_str(&s).ok());
                        let category = match cat_str.as_str() {
                            "core" => ProcessCategory::Core,
                            "coding" => ProcessCategory::Coding,
                            "browser" => ProcessCategory::Browser,
                            "editor" => ProcessCategory::Editor,
                            "voice" => ProcessCategory::Voice,
                            _ => ProcessCategory::Helper,
                        };
                        let state = match state_str.as_str() {
                            "starting" => ProcessLeaseState::Starting,
                            "running" => ProcessLeaseState::Running,
                            "exited" => ProcessLeaseState::Exited,
                            _ => ProcessLeaseState::Unknown,
                        };

                        Ok(ProcessLease {
                            id,
                            job_id,
                            category,
                            pid: pid.map(|p| p as u32),
                            start_ticks: start_ticks.map(|t| t as u64),
                            pgid: pgid.map(|p| p as u32),
                            boot_id,
                            owned: owned != 0,
                            state,
                        })
                    })?
                    .collect::<std::result::Result<Vec<_>, rusqlite::Error>>()?;

                Ok(rows)
            })
            .await?;

        let mut processes = Vec::new();
        let mut category_groups: HashMap<
            ProcessCategory,
            (Vec<u32>, u64, ResourceMeasurementStatus),
        > = HashMap::new();

        for lease in leases {
            if let Some(pid) = lease.pid {
                let (rss, status) = Self::read_process_rss(pid);
                let name = Self::read_process_name(pid).unwrap_or_else(|| "unknown".to_string());

                if include_processes && processes.len() < MAX_PROCESSES_IN_SNAPSHOT {
                    processes.push(ProcessInfo {
                        pid,
                        name,
                        rss_bytes: rss,
                        cpu_percent_one_core: None,
                        category: lease.category,
                        owned: lease.owned,
                    });
                }

                let entry = category_groups.entry(lease.category).or_insert((
                    Vec::new(),
                    0,
                    ResourceMeasurementStatus::Ok,
                ));
                entry.0.push(pid);
                if let Some(r) = rss {
                    entry.1 += r;
                } else if status != ResourceMeasurementStatus::Ok {
                    entry.2 = ResourceMeasurementStatus::Partial;
                }
            }
        }

        let processes_truncated = processes.len() >= MAX_PROCESSES_IN_SNAPSHOT;

        let mut external = Vec::new();
        for category in [
            ProcessCategory::Coding,
            ProcessCategory::Browser,
            ProcessCategory::Editor,
            ProcessCategory::Voice,
            ProcessCategory::Helper,
        ] {
            if let Some((pids, total_rss, status)) = category_groups.remove(&category) {
                external.push(ResourceGroup {
                    category,
                    pids,
                    rss_bytes: Some(total_rss),
                    cpu_percent_one_core: None,
                    measurement_status: status,
                });
            }
        }

        // 3. System resources from /proc/meminfo, statvfs, and /proc directory count
        let (total_ram_bytes, available_ram_bytes) = Self::read_system_ram();
        let (storage_total_bytes, storage_available_bytes) =
            Self::read_storage_stats(&self.config.data_dir);
        let process_count = Self::count_system_processes();

        if available_ram_bytes < LOW_AVAILABLE_RAM_WARN_BYTES && available_ram_bytes > 0 {
            warnings.push(format!(
                "Low available system RAM: {} MB",
                available_ram_bytes / (1024 * 1024)
            ));
        }

        if storage_available_bytes < LOW_STORAGE_WARN_BYTES && storage_available_bytes > 0 {
            warnings.push(format!(
                "Low available storage: {} MB",
                storage_available_bytes / (1024 * 1024)
            ));
        }

        let system = SystemResources {
            total_ram_bytes,
            available_ram_bytes,
            storage_total_bytes,
            storage_available_bytes,
            process_count,
        };

        Ok(ResourceSnapshot {
            at: now,
            core: core_group,
            external,
            system,
            warnings,
            processes,
            processes_truncated,
        })
    }

    pub async fn subscribe(
        &self,
        cancel: CancellationToken,
    ) -> Result<broadcast::Receiver<ResourceSnapshot>> {
        let rx = self.tx.subscribe();
        let monitor = self.clone();
        let tx = self.tx.clone();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    _ = interval.tick() => {
                        if tx.receiver_count() == 0 {
                            break;
                        }
                        if let Ok(snapshot) = monitor.sample(false).await {
                            let _ = tx.send(snapshot);
                        }
                    }
                }
            }
        });

        Ok(rx)
    }

    fn read_process_rss(pid: u32) -> (Option<u64>, ResourceMeasurementStatus) {
        let path = format!("/proc/{pid}/statm");
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                let parts: Vec<&str> = content.split_whitespace().collect();
                if parts.len() >= 2 {
                    if let Ok(pages) = parts[1].parse::<u64>() {
                        (Some(pages * PAGE_SIZE_BYTES), ResourceMeasurementStatus::Ok)
                    } else {
                        (None, ResourceMeasurementStatus::Partial)
                    }
                } else {
                    (None, ResourceMeasurementStatus::Partial)
                }
            }
            Err(_) => (None, ResourceMeasurementStatus::Unavailable),
        }
    }

    fn read_process_name(pid: u32) -> Option<String> {
        let path = format!("/proc/{pid}/comm");
        std::fs::read_to_string(&path)
            .ok()
            .map(|s| s.trim().to_string())
    }

    fn read_system_ram() -> (u64, u64) {
        let mut total_kb = 0u64;
        let mut available_kb = 0u64;

        if let Ok(meminfo) = std::fs::read_to_string("/proc/meminfo") {
            for line in meminfo.lines() {
                if line.starts_with("MemTotal:") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 2 {
                        total_kb = parts[1].parse::<u64>().unwrap_or(0);
                    }
                } else if line.starts_with("MemAvailable:") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 2 {
                        available_kb = parts[1].parse::<u64>().unwrap_or(0);
                    }
                }
            }
        }

        (total_kb * 1024, available_kb * 1024)
    }

    fn read_storage_stats(dir: &Path) -> (u64, u64) {
        match rustix::fs::statvfs(dir) {
            Ok(stat) => {
                let total = stat.f_blocks * stat.f_frsize;
                let available = stat.f_bavail * stat.f_frsize;
                (total, available)
            }
            Err(_) => (0, 0),
        }
    }

    fn count_system_processes() -> u32 {
        let mut count = 0u32;
        if let Ok(entries) = std::fs::read_dir("/proc") {
            for entry in entries.flatten() {
                if let Ok(file_name) = entry.file_name().into_string()
                    && file_name.chars().all(|c| c.is_ascii_digit())
                {
                    count += 1;
                }
            }
        }
        count
    }
}
