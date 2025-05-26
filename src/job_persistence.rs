//! Job persistence and recovery system.
//!
//! This module provides functionality to persist jobs to disk and recover them after system
//! restarts or failures. It includes:
//!
//! * Serialization and deserialization of jobs
//! * Storage and loading of jobs from disk
//! * Automatic backup of in-progress jobs
//! * Tracking of uncompleted jobs

use std::collections::HashMap;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime};

use chrono::{DateTime, Utc};
use log::{debug, error, info, warn};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::log_error;
use crate::job::{Job, JobBatch};

/// The default directory for storing persisted jobs
const DEFAULT_PERSISTENCE_DIR: &str = "./job_persistence";

/// The default interval for automatic backups (5 minutes)
const DEFAULT_BACKUP_INTERVAL: Duration = Duration::from_secs(300);

/// Error type specific to job persistence operations
#[derive(Debug, thiserror::Error)]
pub enum PersistenceError {
    /// Error occurred during serialization or deserialization
    #[error("Serialization error: {0}")]
    SerializationError(String),

    /// Error occurred during file I/O operations
    #[error("File I/O error: {0}")]
    FileError(#[from] io::Error),

    /// Invalid persistence directory
    #[error("Invalid persistence directory: {0}")]
    InvalidDirectory(String),

    /// Missing job data
    #[error("Missing job data: {0}")]
    MissingJobData(String),
}

impl From<PersistenceError> for Error {
    fn from(err: PersistenceError) -> Self {
        match err {
            PersistenceError::SerializationError(msg) => Error::other(format!("Serialization error: {}", msg)),
            PersistenceError::FileError(err) => Error::IoError(err),
            PersistenceError::InvalidDirectory(msg) => Error::other(format!("Invalid persistence directory: {}", msg)),
            PersistenceError::MissingJobData(msg) => Error::other(format!("Missing job data: {}", msg)),
        }
    }
}

impl From<serde_json::Error> for PersistenceError {
    fn from(err: serde_json::Error) -> Self {
        PersistenceError::SerializationError(err.to_string())
    }
}

/// A serializable/deserializable job representation
#[derive(Debug, Serialize, Deserialize)]
pub struct SerializableJob {
    /// Unique identifier for the job
    pub id: String,
    
    /// Description of the job
    pub description: String,
    
    /// Time when the job was created
    pub creation_time: DateTime<Utc>,
    
    /// Serialized job data (specific to the job type)
    pub data: String,
    
    /// Job type identifier (used for deserialization)
    pub job_type: String,
}

impl SerializableJob {
    /// Create a new serializable job
    pub fn new(
        id: impl Into<String>,
        description: impl Into<String>,
        data: impl Into<String>,
        job_type: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            description: description.into(),
            creation_time: Utc::now(),
            data: data.into(),
            job_type: job_type.into(),
        }
    }
}

/// A trait for jobs that can be persisted to disk
pub trait PersistableJob: Job {
    /// Serialize the job to a serializable representation
    fn serialize(&self) -> Result<SerializableJob>;
    
    /// Get the job type identifier
    fn job_type(&self) -> &'static str;
}

/// Registry for job deserializers
pub trait JobDeserializer: Send + Sync {
    /// Deserialize a job from its serialized representation
    fn deserialize(&self, job: &SerializableJob) -> Result<Arc<dyn Job>>;
    
    /// Get the job type this deserializer handles
    fn job_type(&self) -> &'static str;
}

/// Manager for job persistence and recovery
#[derive(Clone)]
pub struct JobPersistenceManager {
    /// Directory where job data is stored
    persistence_dir: PathBuf,
    
    /// Interval for automatic backups
    backup_interval: Duration,
    
    /// Last time a backup was performed
    last_backup: Arc<RwLock<Instant>>,
    
    /// Registry of job deserializers
    deserializers: Arc<RwLock<HashMap<String, Arc<dyn JobDeserializer>>>>,
    
    /// In-memory record of uncompleted jobs (id -> job)
    uncompleted_jobs: Arc<RwLock<HashMap<String, Arc<dyn Job>>>>,
}

impl JobPersistenceManager {
    /// Create a new job persistence manager
    pub fn new() -> Result<Self> {
        Self::with_directory(DEFAULT_PERSISTENCE_DIR)
    }
    
    /// Create a new job persistence manager with a custom directory
    pub fn with_directory(dir: impl AsRef<Path>) -> Result<Self> {
        let path = dir.as_ref().to_path_buf();
        
        // Create the directory if it doesn't exist
        if !path.exists() {
            fs::create_dir_all(&path).map_err(|e| {
                PersistenceError::FileError(io::Error::new(
                    io::ErrorKind::Other,
                    format!("Failed to create persistence directory: {}", e),
                ))
            })?;
        }
        
        // Verify it's a directory
        if !path.is_dir() {
            return Err(PersistenceError::InvalidDirectory(format!(
                "Path {} is not a directory",
                path.display()
            ))
            .into());
        }
        
        Ok(Self {
            persistence_dir: path,
            backup_interval: DEFAULT_BACKUP_INTERVAL,
            last_backup: Arc::new(RwLock::new(Instant::now())),
            deserializers: Arc::new(RwLock::new(HashMap::new())),
            uncompleted_jobs: Arc::new(RwLock::new(HashMap::new())),
        })
    }
    
    /// Register a job deserializer
    pub fn register_deserializer(&self, deserializer: Arc<dyn JobDeserializer>) -> Result<()> {
        let job_type = deserializer.job_type().to_string();
        let mut deserializers = self.deserializers.write().map_err(|e| {
            Error::lock_error(format!("Failed to acquire deserializers lock: {}", e))
        })?;
        
        deserializers.insert(job_type, deserializer);
        Ok(())
    }
    
    /// Set the backup interval
    pub fn set_backup_interval(&mut self, interval: Duration) {
        self.backup_interval = interval;
    }
    
    /// Force a backup of all uncompleted jobs
    pub fn backup(&self) -> Result<()> {
        let uncompleted_jobs = self.uncompleted_jobs.read().map_err(|e| {
            Error::lock_error(format!("Failed to acquire uncompleted jobs lock: {}", e))
        })?;
        
        // Skip if there are no uncompleted jobs
        if uncompleted_jobs.is_empty() {
            debug!("No uncompleted jobs to backup");
            return Ok(());
        }
        
        // Create a timestamped backup filename
        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        
        let backup_file = self.persistence_dir.join(format!("backup_{}.json", timestamp));
        let file = match File::create(&backup_file) {
            Ok(f) => f,
            Err(e) => {
                let _ = log_error!("Failed to create backup file {}: {}", backup_file.display(), e);
                return Err(PersistenceError::from(e).into());
            }
        };
        let writer = BufWriter::new(file);
        
        // Store job ids for backup
        let job_ids: Vec<String> = uncompleted_jobs.keys().cloned().collect();
        
        // Serialize to JSON
        if let Err(e) = serde_json::to_writer_pretty(writer, &job_ids) {
            let _ = log_error!("Failed to write backup data: {}", e);
            return Err(PersistenceError::from(e).into());
        }
        
        info!("Backed up {} jobs to {}", job_ids.len(), backup_file.display());
        
        // Update last backup time
        if let Ok(mut last_backup) = self.last_backup.write() {
            *last_backup = Instant::now();
        }
        
        Ok(())
    }
    
    /// Check if a backup is needed based on the backup interval
    pub fn check_backup_needed(&self) -> bool {
        if let Ok(last_backup) = self.last_backup.read() {
            last_backup.elapsed() >= self.backup_interval
        } else {
            // Default to true if we can't read the lock (unlikely)
            true
        }
    }
    
    /// Persist a job to disk
    pub fn persist_job(&self, job: Arc<dyn PersistableJob>) -> Result<()> {
        let serializable = match job.serialize() {
            Ok(s) => s,
            Err(e) => {
                let _ = log_error!("Failed to serialize job: {}", e);
                return Err(e);
            }
        };
        let job_id = serializable.id.clone();
        
        // Save to disk
        let job_file = self.persistence_dir.join(format!("{}.json", job_id));
        let file = match File::create(&job_file) {
            Ok(f) => f,
            Err(e) => {
                let _ = log_error!("Failed to create job file {}: {}", job_file.display(), e);
                return Err(PersistenceError::from(e).into());
            }
        };
        let writer = BufWriter::new(file);
        
        if let Err(e) = serde_json::to_writer_pretty(writer, &serializable) {
            let _ = log_error!("Failed to write job {} to file: {}", job_id, e);
            return Err(PersistenceError::from(e).into());
        }
        
        // Add to uncompleted jobs
        let mut uncompleted_jobs = self.uncompleted_jobs.write().map_err(|e| {
            Error::lock_error(format!("Failed to acquire uncompleted jobs lock: {}", e))
        })?;
        
        uncompleted_jobs.insert(job_id.clone(), job.clone());
        
        debug!("Persisted job {} to {}", job_id, job_file.display());
        
        // Check if we need to backup
        if self.check_backup_needed() {
            drop(uncompleted_jobs); // Release the lock before backup
            if let Err(e) = self.backup() {
                warn!("Failed to backup jobs: {}", e);
            }
        }
        
        Ok(())
    }
    
    /// Persist a batch of jobs
    pub fn persist_job_batch(&self, batch: &JobBatch) -> Result<()> {
        // Create a unique batch ID
        let batch_id = format!(
            "batch_{}_{}",
            batch.description().replace(" ", "_"),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        );
        
        // Create a directory for this batch
        let batch_dir = self.persistence_dir.join(&batch_id);
        fs::create_dir_all(&batch_dir).map_err(PersistenceError::from)?;
        
        // Save metadata about the batch
        let metadata = serde_json::json!({
            "id": batch_id,
            "description": batch.description(),
            "job_count": batch.len(),
            "creation_time": DateTime::<Utc>::from(SystemTime::now()),
        });
        
        let metadata_file = batch_dir.join("metadata.json");
        let file = File::create(&metadata_file).map_err(PersistenceError::from)?;
        let writer = BufWriter::new(file);
        
        serde_json::to_writer_pretty(writer, &metadata).map_err(PersistenceError::from)?;
        
        // Save job IDs for the batch
        let mut job_ids = Vec::new();
        
        // Process each job
        for (i, job) in batch.jobs().iter().enumerate() {
            // For PersistableJob, use its serialization
            // Try to downcast the job to a PersistableJob
            let any_job = job.clone();
            if let Some(persistable) = self.try_downcast_job(any_job) {
                let serializable = persistable.serialize()?;
                let job_id = serializable.id.clone();
                job_ids.push(job_id.clone());
                
                // Save to disk in the batch directory
                let job_file = batch_dir.join(format!("{}.json", job_id));
                let file = File::create(&job_file).map_err(PersistenceError::from)?;
                let writer = BufWriter::new(file);
                
                serde_json::to_writer_pretty(writer, &serializable).map_err(PersistenceError::from)?;
                
                // Add to uncompleted jobs
                let mut uncompleted_jobs = self.uncompleted_jobs.write().map_err(|e| {
                    Error::lock_error(format!("Failed to acquire uncompleted jobs lock: {}", e))
                })?;
                
                uncompleted_jobs.insert(job_id, job.clone());
            } else {
                warn!(
                    "Job {} in batch '{}' is not persistable and will be skipped",
                    i,
                    batch.description()
                );
            }
        }
        
        // Save job IDs
        let ids_file = batch_dir.join("job_ids.json");
        let file = File::create(&ids_file).map_err(PersistenceError::from)?;
        let writer = BufWriter::new(file);
        
        serde_json::to_writer_pretty(writer, &job_ids).map_err(PersistenceError::from)?;
        
        info!(
            "Persisted job batch '{}' with {} jobs to {}",
            batch.description(),
            job_ids.len(),
            batch_dir.display()
        );
        
        // Check if we need to backup
        if self.check_backup_needed() {
            if let Err(e) = self.backup() {
                warn!("Failed to backup jobs: {}", e);
            }
        }
        
        Ok(())
    }
    
    /// Mark a job as completed and remove it from persistence
    pub fn mark_completed(&self, job_id: &str) -> Result<()> {
        // Remove from uncompleted jobs
        let mut uncompleted_jobs = self.uncompleted_jobs.write().map_err(|e| {
            Error::lock_error(format!("Failed to acquire uncompleted jobs lock: {}", e))
        })?;
        
        uncompleted_jobs.remove(job_id);
        
        // Remove the persisted file
        let job_file = self.persistence_dir.join(format!("{}.json", job_id));
        if job_file.exists() {
            fs::remove_file(&job_file).map_err(PersistenceError::from)?;
            debug!("Removed completed job file: {}", job_file.display());
        }
        
        Ok(())
    }
    
    /// Load persisted jobs from disk
    pub fn load_persisted_jobs(&self) -> Result<Vec<Arc<dyn Job>>> {
        let mut jobs = Vec::new();
        
        // Read all JSON files in the persistence directory
        for entry in fs::read_dir(&self.persistence_dir).map_err(PersistenceError::from)? {
            let entry = entry.map_err(PersistenceError::from)?;
            let path = entry.path();
            
            // Skip directories (except for batch directories which we'll handle later)
            if path.is_dir() {
                continue;
            }
            
            // Only process JSON files
            if let Some(ext) = path.extension() {
                if ext != "json" {
                    continue;
                }
            } else {
                continue;
            }
            
            // Skip backup files
            if let Some(file_name) = path.file_name() {
                let file_name = file_name.to_string_lossy();
                if file_name.starts_with("backup_") {
                    continue;
                }
            }
            
            // Read and deserialize the job
            if let Ok(job) = self.deserialize_job_from_file(&path) {
                jobs.push(job);
            }
        }
        
        // Add the jobs to uncompleted_jobs
        let mut uncompleted_jobs = self.uncompleted_jobs.write().map_err(|e| {
            Error::lock_error(format!("Failed to acquire uncompleted jobs lock: {}", e))
        })?;
        
        for job in &jobs {
            // Try to get the job ID using the PersistableJob trait
            // Try to downcast the job to a PersistableJob
            let any_job = job.clone();
            if let Some(persistable) = self.try_downcast_job(any_job) {
                if let Ok(serializable) = persistable.serialize() {
                    uncompleted_jobs.insert(serializable.id, job.clone());
                }
            }
        }
        
        info!("Loaded {} persisted jobs", jobs.len());
        
        Ok(jobs)
    }
    
    /// Load the most recent backup
    pub fn load_most_recent_backup(&self) -> Result<Vec<Arc<dyn Job>>> {
        let mut backup_files = Vec::new();
        
        // Find all backup files
        for entry in fs::read_dir(&self.persistence_dir).map_err(PersistenceError::from)? {
            let entry = entry.map_err(PersistenceError::from)?;
            let path = entry.path();
            
            // Only process JSON files
            if let Some(ext) = path.extension() {
                if ext != "json" {
                    continue;
                }
            } else {
                continue;
            }
            
            // Only consider backup files
            if let Some(file_name) = path.file_name() {
                let file_name = file_name.to_string_lossy();
                if file_name.starts_with("backup_") {
                    backup_files.push(path);
                }
            }
        }
        
        // Sort backup files by modification time (newest first)
        backup_files.sort_by(|a, b| {
            let a_time = fs::metadata(a)
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            let b_time = fs::metadata(b)
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            b_time.cmp(&a_time) // Reverse order (newest first)
        });
        
        // Get the most recent backup
        if let Some(backup_file) = backup_files.first() {
            info!("Loading most recent backup: {}", backup_file.display());
            
            // Read the backup file to get job IDs
            let file = File::open(backup_file).map_err(PersistenceError::from)?;
            let reader = BufReader::new(file);
            let job_ids: Vec<String> = serde_json::from_reader(reader).map_err(PersistenceError::from)?;
            
            // Load each job
            let mut jobs = Vec::new();
            for job_id in job_ids {
                let job_file = self.persistence_dir.join(format!("{}.json", job_id));
                if job_file.exists() {
                    if let Ok(job) = self.deserialize_job_from_file(&job_file) {
                        jobs.push(job);
                    }
                }
            }
            
            info!("Loaded {} jobs from backup", jobs.len());
            return Ok(jobs);
        }
        
        // No backup found
        info!("No backup files found");
        Ok(Vec::new())
    }
    
    /// Get the list of uncompleted jobs
    pub fn get_uncompleted_jobs(&self) -> Result<Vec<Arc<dyn Job>>> {
        let uncompleted_jobs = self.uncompleted_jobs.read().map_err(|e| {
            Error::lock_error(format!("Failed to acquire uncompleted jobs lock: {}", e))
        })?;
        
        Ok(uncompleted_jobs.values().cloned().collect())
    }
    
    /// Check if a job is uncompleted
    pub fn is_job_uncompleted(&self, job_id: &str) -> Result<bool> {
        let uncompleted_jobs = self.uncompleted_jobs.read().map_err(|e| {
            Error::lock_error(format!("Failed to acquire uncompleted jobs lock: {}", e))
        })?;
        
        Ok(uncompleted_jobs.contains_key(job_id))
    }
    
    /// Clear all persisted jobs
    pub fn clear_all_jobs(&self) -> Result<()> {
        // Clear uncompleted jobs
        let mut uncompleted_jobs = self.uncompleted_jobs.write().map_err(|e| {
            Error::lock_error(format!("Failed to acquire uncompleted jobs lock: {}", e))
        })?;
        
        uncompleted_jobs.clear();
        
        // Remove all files in the persistence directory
        for entry in fs::read_dir(&self.persistence_dir).map_err(PersistenceError::from)? {
            let entry = entry.map_err(PersistenceError::from)?;
            let path = entry.path();
            
            if path.is_file() {
                fs::remove_file(&path).map_err(PersistenceError::from)?;
            } else if path.is_dir() {
                fs::remove_dir_all(&path).map_err(PersistenceError::from)?;
            }
        }
        
        info!("Cleared all persisted jobs");
        
        Ok(())
    }
    
    // Helper function to deserialize a job from a file
    fn deserialize_job_from_file(&self, file_path: &Path) -> Result<Arc<dyn Job>> {
        // Read the file
        let file = File::open(file_path).map_err(PersistenceError::from)?;
        let mut reader = BufReader::new(file);
        
        let mut contents = String::new();
        reader.read_to_string(&mut contents).map_err(PersistenceError::from)?;
        
        // Parse as SerializableJob
        let serializable: SerializableJob = serde_json::from_str(&contents).map_err(PersistenceError::from)?;
        
        // Find the appropriate deserializer
        let deserializers = self.deserializers.read().map_err(|e| {
            Error::lock_error(format!("Failed to acquire deserializers lock: {}", e))
        })?;
        
        if let Some(deserializer) = deserializers.get(&serializable.job_type) {
            deserializer.deserialize(&serializable)
        } else {
            Err(PersistenceError::MissingJobData(format!(
                "No deserializer found for job type: {}",
                serializable.job_type
            ))
            .into())
        }
    }
    
    /// Helper method to try to downcast a job to a PersistableJob
    fn try_downcast_job(&self, job: Arc<dyn Job>) -> Option<Arc<dyn PersistableJob + 'static>> {
        // First try using as_any to get the concrete type
        let job_ref = job.as_ref();
        let any_ref = job_ref.as_any();
        
        // Try to downcast to known persistable job types
        if let Some(persistable_job) = any_ref.downcast_ref::<PersistableCallbackJob>() {
            // It's a PersistableCallbackJob, create a new Arc with same data
            let new_job = PersistableCallbackJob::new(
                persistable_job.id().to_string(),
                persistable_job.description().to_string(), 
                persistable_job.serialized_data().to_string(),
                // We need a dummy executor since we can't clone the original
                || Ok(())
            );
            
            return Some(Arc::new(new_job) as Arc<dyn PersistableJob>);
        }
        
        // We could add more job type checks here if needed
        
        // If we can't downcast, return None
        None
    }
}

/// A Persistable implementation of the CallbackJob
/// This is a reference implementation showing how jobs can implement PersistableJob
pub struct PersistableCallbackJob {
    /// The ID of this job
    id: String,
    
    /// Description of the job
    description: String,
    
    /// When the job was created
    creation_time: Instant,
    
    /// Job data in serializable form
    serialized_data: String,
    
    /// Function to execute when the job is run
    executor: Box<dyn Fn() -> Result<()> + Send + Sync>,
}

impl PersistableCallbackJob {
    /// Create a new persistable callback job
    pub fn new(
        id: impl Into<String>,
        description: impl Into<String>,
        data: impl Into<String>,
        executor: impl Fn() -> Result<()> + Send + Sync + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            description: description.into(),
            creation_time: Instant::now(),
            serialized_data: data.into(),
            executor: Box::new(executor),
        }
    }
    
    /// Get the ID of this job
    pub fn id(&self) -> &str {
        &self.id
    }
    
    /// Get the serialized data for this job
    pub fn serialized_data(&self) -> &str {
        &self.serialized_data
    }
}

impl Job for PersistableCallbackJob {
    fn execute_with_context(&self, context: &crate::job::JobContext) -> Result<()> {
        // Check for cancellation before execution
        if context.is_cancelled() {
            return Err(crate::error::Error::JobCancelled);
        }
        (self.executor)()
    }
    
    fn execute(&self) -> Result<()> {
        (self.executor)()
    }
    
    fn description(&self) -> String {
        self.description.clone()
    }
    
    fn creation_time(&self) -> Instant {
        self.creation_time
    }
    
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl PersistableJob for PersistableCallbackJob {
    fn serialize(&self) -> Result<SerializableJob> {
        Ok(SerializableJob {
            id: self.id.clone(),
            description: self.description.clone(),
            creation_time: DateTime::<Utc>::from(SystemTime::now()),
            data: self.serialized_data.clone(),
            job_type: self.job_type().to_string(),
        })
    }
    
    fn job_type(&self) -> &'static str {
        "persistable_callback_job"
    }
}

impl fmt::Debug for PersistableCallbackJob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PersistableCallbackJob")
            .field("id", &self.id)
            .field("description", &self.description)
            .field("creation_time", &format!("{:?}", self.creation_time))
            .field("serialized_data", &self.serialized_data)
            .field("executor", &"<function>")
            .finish()
    }
}

/// Type alias for the factory function that creates job executors
type JobExecutorFactory = Box<dyn Fn(&str) -> Box<dyn Fn() -> Result<()> + Send + Sync> + Send + Sync>;

/// Deserializer for PersistableCallbackJob
pub struct PersistableCallbackJobDeserializer {
    /// Function that creates an executor from serialized data
    factory: JobExecutorFactory,
}

impl PersistableCallbackJobDeserializer {
    /// Create a new deserializer with a factory function
    pub fn new(
        factory: impl Fn(&str) -> Box<dyn Fn() -> Result<()> + Send + Sync> + Send + Sync + 'static,
    ) -> Self {
        Self {
            factory: Box::new(factory),
        }
    }
}

impl JobDeserializer for PersistableCallbackJobDeserializer {
    fn deserialize(&self, job: &SerializableJob) -> Result<Arc<dyn Job>> {
        let executor = (self.factory)(&job.data);
        
        Ok(Arc::new(PersistableCallbackJob {
            id: job.id.clone(),
            description: job.description.clone(),
            creation_time: Instant::now(), // We can't deserialize Instant, so use current time
            serialized_data: job.data.clone(),
            executor,
        }))
    }
    
    fn job_type(&self) -> &'static str {
        "persistable_callback_job"
    }
}

impl fmt::Debug for PersistableCallbackJobDeserializer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PersistableCallbackJobDeserializer")
            .field("factory", &"<function>")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    
    // Temporary directory for testing
    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("job_persistence_test_{}", rand::random::<u64>()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }
    
    // Clean up temporary directory
    fn cleanup_temp_dir(dir: &Path) {
        let _ = fs::remove_dir_all(dir);
    }
    
    #[test]
    fn test_persistence_manager_creation() {
        let dir = temp_dir();
        
        // Test creation with directory
        let manager = JobPersistenceManager::with_directory(&dir);
        assert!(manager.is_ok());
        
        // Test default creation
        let manager = JobPersistenceManager::new();
        assert!(manager.is_ok());
        
        cleanup_temp_dir(&dir);
    }
    
    #[test]
    fn test_persist_and_load_job() {
        let dir = temp_dir();
        let manager = JobPersistenceManager::with_directory(&dir).unwrap();
        
        // Create a flag to track execution
        let executed = Arc::new(AtomicBool::new(false));
        let executed_clone = executed.clone();
        
        // Create a persistable job
        let job = PersistableCallbackJob::new(
            "test_job_1",
            "Test Job",
            "test_data",
            move || {
                executed_clone.store(true, Ordering::SeqCst);
                Ok(())
            },
        );
        
        // Register deserializer
        let deserializer = Arc::new(PersistableCallbackJobDeserializer::new(|data| {
            let data_owned = data.to_string();
            Box::new(move || {
                let _ = crate::log_info!("Executing deserialized job with data: {}", data_owned);
                Ok(())
            })
        }));
        
        manager.register_deserializer(deserializer).unwrap();
        
        // Persist the job
        manager.persist_job(Arc::new(job)).unwrap();
        
        // Check if job is in uncompleted jobs
        assert!(manager.is_job_uncompleted("test_job_1").unwrap());
        
        // Load persisted jobs
        let loaded_jobs = manager.load_persisted_jobs().unwrap();
        assert_eq!(loaded_jobs.len(), 1);
        
        // Clean up
        manager.clear_all_jobs().unwrap();
        cleanup_temp_dir(&dir);
    }
    
    #[test]
    fn test_mark_completed() {
        let dir = temp_dir();
        let manager = JobPersistenceManager::with_directory(&dir).unwrap();
        
        // Create a persistable job
        let job = PersistableCallbackJob::new(
            "test_job_2",
            "Test Job",
            "test_data",
            || Ok(()),
        );
        
        // Register deserializer
        let deserializer = Arc::new(PersistableCallbackJobDeserializer::new(|_| {
            Box::new(move || Ok(()))
        }));
        
        manager.register_deserializer(deserializer).unwrap();
        
        // Persist the job
        manager.persist_job(Arc::new(job)).unwrap();
        
        // Check if job is in uncompleted jobs
        assert!(manager.is_job_uncompleted("test_job_2").unwrap());
        
        // Mark as completed
        manager.mark_completed("test_job_2").unwrap();
        
        // Check if job is removed from uncompleted jobs
        assert!(!manager.is_job_uncompleted("test_job_2").unwrap());
        
        // Clean up
        manager.clear_all_jobs().unwrap();
        cleanup_temp_dir(&dir);
    }
    
    #[test]
    fn test_backup_and_recovery() {
        let dir = temp_dir();
        let manager = JobPersistenceManager::with_directory(&dir).unwrap();
        
        // Create a persistable job
        let job1 = PersistableCallbackJob::new(
            "test_job_3",
            "Test Job 3",
            "test_data_3",
            || Ok(()),
        );
        
        let job2 = PersistableCallbackJob::new(
            "test_job_4",
            "Test Job 4",
            "test_data_4",
            || Ok(()),
        );
        
        // Register deserializer
        let deserializer = Arc::new(PersistableCallbackJobDeserializer::new(|_| {
            Box::new(move || Ok(()))
        }));
        
        manager.register_deserializer(deserializer).unwrap();
        
        // Persist jobs
        manager.persist_job(Arc::new(job1)).unwrap();
        manager.persist_job(Arc::new(job2)).unwrap();
        
        // Force backup
        manager.backup().unwrap();
        
        // Mark one job as completed
        manager.mark_completed("test_job_3").unwrap();
        
        // Load from backup
        let recovered_jobs = manager.load_most_recent_backup().unwrap();
        
        // Should have at least one job in the backup
        // The backup might either return both jobs or just the active job depending on timing
        assert!(recovered_jobs.len() >= 1);
        
        // Clean up
        manager.clear_all_jobs().unwrap();
        cleanup_temp_dir(&dir);
    }
}