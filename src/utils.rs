//! Utility functions and types for the thread system
//!
//! This module provides various utility functions and types that are used throughout
//! the thread system but don't fit into any specific category.

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::error::{Error, Result};

/// Get the current timestamp as a formatted string.
pub fn get_timestamp() -> String {
    let now = chrono::Local::now();
    now.format("%Y-%m-%d %H:%M:%S%.3f").to_string()
}

/// Get the current time as milliseconds since the UNIX epoch.
pub fn get_time_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_millis()
}

/// Parse a duration from a string.
///
/// Supports formats like "100ms", "5s", "1m", "2h" and plain milliseconds.
pub fn parse_duration(s: &str) -> Result<Duration> {
    let s = s.trim().to_lowercase();
    
    let (num_str, multiplier) = if s.ends_with("ms") {
        (&s[..s.len() - 2], 1)
    } else if s.ends_with('s') {
        (&s[..s.len() - 1], 1000)
    } else if s.ends_with('m') {
        (&s[..s.len() - 1], 60 * 1000)
    } else if s.ends_with('h') {
        (&s[..s.len() - 1], 60 * 60 * 1000)
    } else {
        (&s[..], 1) // Assume milliseconds
    };
    
    let value = num_str.parse::<u64>().map_err(|_| 
        Error::other(format!("Failed to parse duration: {}", s))
    )?;
    
    Ok(Duration::from_millis(value * multiplier))
}

/// Command line argument parser.
#[derive(Debug)]
pub struct ArgumentParser {
    /// Program name
    program_name: String,
    /// Program description
    description: String,
    /// Command line arguments
    args: Vec<String>,
}

impl ArgumentParser {
    /// Create a new argument parser.
    pub fn new(program_name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            program_name: program_name.into(),
            description: description.into(),
            args: std::env::args().collect(),
        }
    }
    
    /// Get the value of a flag argument.
    ///
    /// Returns the value that follows the flag if present.
    pub fn get_value(&self, flag: &str) -> Option<String> {
        for i in 0..self.args.len() {
            if self.args[i] == flag && i + 1 < self.args.len() {
                return Some(self.args[i + 1].clone());
            }
        }
        None
    }
    
    /// Check if a flag is present in the arguments.
    pub fn has_flag(&self, flag: &str) -> bool {
        self.args.iter().any(|arg| arg == flag)
    }
    
    /// Get the total number of arguments.
    pub fn argument_count(&self) -> usize {
        self.args.len()
    }
    
    /// Print usage information to the console.
    pub fn print_usage(&self) {
        let _ = crate::log_info!("{}", self.program_name);
        let _ = crate::log_info!("{}", self.description);
        let _ = crate::log_info!("Usage: {} [options]", self.args[0]);
    }
}

/// File utilities for common file operations.
pub struct FileUtils;

impl FileUtils {
    /// Check if a file exists.
    pub fn file_exists(path: impl AsRef<Path>) -> bool {
        path.as_ref().is_file()
    }
    
    /// Check if a directory exists.
    pub fn directory_exists(path: impl AsRef<Path>) -> bool {
        path.as_ref().is_dir()
    }
    
    /// Create a directory and all parent directories.
    pub fn create_directory(path: impl AsRef<Path>) -> Result<()> {
        std::fs::create_dir_all(path.as_ref()).map_err(Error::from)
    }
    
    /// Get the size of a file in bytes.
    pub fn file_size(path: impl AsRef<Path>) -> Result<u64> {
        std::fs::metadata(path.as_ref())
            .map(|m| m.len())
            .map_err(Error::from)
    }
    
    /// Read a file as a string.
    pub fn read_to_string(path: impl AsRef<Path>) -> Result<String> {
        std::fs::read_to_string(path.as_ref()).map_err(Error::from)
    }
    
    /// Write a string to a file.
    pub fn write_string(path: impl AsRef<Path>, content: impl AsRef<str>) -> Result<()> {
        std::fs::write(path.as_ref(), content.as_ref()).map_err(Error::from)
    }
}

/// Date and time utilities.
pub struct DateTimeUtils;

impl DateTimeUtils {
    /// Get the current date and time as a formatted string.
    pub fn now_string(format: &str) -> String {
        chrono::Local::now().format(format).to_string()
    }
    
    /// Get the current date as a string (YYYY-MM-DD).
    pub fn current_date() -> String {
        Self::now_string("%Y-%m-%d")
    }
    
    /// Get the current time as a string (HH:MM:SS).
    pub fn current_time() -> String {
        Self::now_string("%H:%M:%S")
    }
    
    /// Get the current date and time as a string (YYYY-MM-DD HH:MM:SS).
    pub fn current_datetime() -> String {
        Self::now_string("%Y-%m-%d %H:%M:%S")
    }
}

/// String utilities.
pub struct StringUtils;

impl StringUtils {
    /// Check if a string is empty or only contains whitespace.
    pub fn is_empty_or_whitespace(s: &str) -> bool {
        s.trim().is_empty()
    }
    
    /// Truncate a string to a maximum length, adding an ellipsis if truncated.
    pub fn truncate(s: &str, max_length: usize) -> String {
        if s.len() <= max_length {
            s.to_string()
        } else {
            let mut result = s[..max_length - 3].to_string();
            result.push_str("...");
            result
        }
    }
    
    /// Split a string into lines and trim each line.
    pub fn split_and_trim(s: &str) -> Vec<String> {
        s.lines()
            .map(|line| line.trim().to_string())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_parse_duration() {
        assert_eq!(parse_duration("100ms").unwrap(), Duration::from_millis(100));
        assert_eq!(parse_duration("5s").unwrap(), Duration::from_millis(5000));
        assert_eq!(parse_duration("2m").unwrap(), Duration::from_millis(2 * 60 * 1000));
        assert_eq!(parse_duration("1h").unwrap(), Duration::from_millis(60 * 60 * 1000));
        assert_eq!(parse_duration("500").unwrap(), Duration::from_millis(500));
        
        assert!(parse_duration("invalid").is_err());
    }
    
    #[test]
    fn test_argument_parser() {
        let parser = ArgumentParser::new("test_program", "A test program");
        
        assert_eq!(parser.program_name, "test_program");
        assert!(parser.args.len() > 0); // At least the program name should be present
    }
    
    #[test]
    fn test_string_utils() {
        assert!(StringUtils::is_empty_or_whitespace(""));
        assert!(StringUtils::is_empty_or_whitespace("  \t  "));
        assert!(!StringUtils::is_empty_or_whitespace("test"));
        
        assert_eq!(StringUtils::truncate("1234567890", 5), "12...");
        assert_eq!(StringUtils::truncate("12345", 5), "12345");
        
        let lines = StringUtils::split_and_trim("line1\n  line2  \nline3");
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "line1");
        assert_eq!(lines[1], "line2");
        assert_eq!(lines[2], "line3");
    }
    
    #[test]
    fn test_datetime_utils() {
        assert!(!DateTimeUtils::current_date().is_empty());
        assert!(!DateTimeUtils::current_time().is_empty());
        assert!(!DateTimeUtils::current_datetime().is_empty());
    }
}