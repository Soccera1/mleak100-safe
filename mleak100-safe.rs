use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::str::FromStr;

const LEAK_SIZE: usize = 1024 * 100; // 100 KB
const MIN_MEMORY_THRESHOLD: u64 = 100 * 1024 * 1024; // 100 MB in bytes
const CHECK_INTERVAL: Duration = Duration::from_millis(10);

// Global vector to store leaks (wrapped in Arc for thread safety)
struct LeakTracker {
    // We need to keep the Vec<Box<[u8]>> alive to maintain the leak
    leaks: Vec<Box<[u8]>>,
    count: usize,
}

impl LeakTracker {
    fn new() -> Self {
        Self {
            leaks: Vec::new(),
            count: 0,
        }
    }

    fn add_leak(&mut self) {
        // Allocate and initialize memory to prevent optimization
        let leak = vec![b'A'; LEAK_SIZE].into_boxed_slice();
        self.leaks.push(leak);
        self.count += 1;
    }

    fn get_total_leaked(&self) -> usize {
        self.count * LEAK_SIZE
    }

    fn get_leak_count(&self) -> usize {
        self.count
    }
}

fn create_directory(dir_name: &str) -> io::Result<()> {
    if !Path::new(dir_name).exists() {
        fs::create_dir(dir_name)?;
    }
    Ok(())
}

fn get_current_time_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
}

fn get_memory_info() -> io::Result<(u64, u64)> {
    let meminfo = fs::read_to_string("/proc/meminfo")?;
    let mut available_memory = 0;
    let mut free_swap = 0;

    for line in meminfo.lines() {
        if line.starts_with("MemAvailable:") {
            available_memory = parse_mem_line(line);
        } else if line.starts_with("SwapFree:") {
            free_swap = parse_mem_line(line);
        }
    }

    Ok((available_memory, free_swap))
}

fn parse_mem_line(line: &str) -> u64 {
    let parts: Vec<&str> = line.split_whitespace().collect();
    u64::from_str(parts[1]).unwrap_or(0) * 1024 // Convert from kB to bytes
}

fn memory_monitor(should_continue: Arc<AtomicBool>) {
    while should_continue.load(Ordering::Relaxed) {
        match get_memory_info() {
            Ok((available_memory, free_swap)) => {
                let total_available = available_memory + free_swap;

                // Log memory status
                println!(
                    "Available RAM: {} MB, Available Swap: {} MB, Total Available: {} MB",
                    available_memory / 1024 / 1024,
                    free_swap / 1024 / 1024,
                    total_available / 1024 / 1024
                );

                // Check if total available memory is below threshold
                if total_available < MIN_MEMORY_THRESHOLD {
                    println!("Total available memory below threshold! Terminating process...");
                    should_continue.store(false, Ordering::Relaxed);
                    return;
                }
            }
            Err(_) => {
                eprintln!("Failed to read memory info.");
            }
        }

        thread::sleep(CHECK_INTERVAL);
    }
}

fn main() -> io::Result<()> {
    let dir_name = "mleak100.log";
    create_directory(dir_name)?;

    // Generate log file name with timestamp
    let timestamp = get_current_time_millis();
    let log_filename = format!("{}/leak_log_{}.txt", dir_name, timestamp);
    let mut log_file = File::create(&log_filename)?;

    // Create atomic bool for thread synchronization
    let should_continue = Arc::new(AtomicBool::new(true));
    let should_continue_clone = Arc::clone(&should_continue);

    // Start memory monitoring thread
    let monitor_thread = thread::spawn(move || {
        memory_monitor(should_continue_clone);
    });

    // Create leak tracker
    let mut leak_tracker = LeakTracker::new();

    // Main loop: keep leaking memory until threshold is reached
    while should_continue.load(Ordering::Relaxed) {
        leak_tracker.add_leak();

        // Log the memory leak
        let log_message = format!(
            "Leaked 100KB of memory (Total leaks: {})\n",
            leak_tracker.get_leak_count()
        );

        // Display to console and write to file
        print!("{}", log_message);
        log_file.write_all(log_message.as_bytes())?;
        log_file.flush()?;
    }

    // Wait for monitor thread to finish
    monitor_thread.join().unwrap();

    // Get final memory stats
    match get_memory_info() {
        Ok((available_memory, free_swap)) => {
            let total_available = available_memory + free_swap;
            let total_leaked = leak_tracker.get_total_leaked();

            // Create final statistics message
            let stats = format!(
                "\nFinal Statistics:\n\
                ----------------\n\
                Available RAM: {} MB\n\
                Available Swap: {} MB\n\
                Total Available: {} MB\n\
                Total Memory Leaked: {} MB ({} allocations of {}KB each)\n",
                available_memory / 1024 / 1024,
                free_swap / 1024 / 1024,
                total_available / 1024 / 1024,
                total_leaked / 1024 / 1024,
                leak_tracker.get_leak_count(),
                LEAK_SIZE / 1024
            );

            // Output to both console and log file
            print!("{}", stats);
            log_file.write_all(stats.as_bytes())?;
            log_file.write_all(b"Process terminated due to memory threshold.\n")?;
        }
        Err(_) => {
            eprintln!("Failed to read final memory stats.");
        }
    }

    Ok(())
}
