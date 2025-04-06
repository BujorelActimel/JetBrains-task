use std::io::{Read, Write, BufWriter};
use std::net::TcpStream;
use std::time::{Instant, Duration};
use std::sync::{Arc, Mutex};
use std::thread;
use std::collections::HashSet;
use std::fs::File;
use std::path::Path;
use sha2::{Sha256, Digest};
use clap::{App, Arg};
use indicatif::{ProgressBar, ProgressStyle, MultiProgress};

struct Chunk {
    id: usize,
    data: Vec<u8>,
}

impl Clone for Chunk {
    fn clone(&self) -> Self {
        Chunk {
            id: self.id,
            data: self.data.clone(),
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let matches = App::new("HTTP Downloader")
        .version("1.0")
        .about("Downloads files from a buggy HTTP server")
        .arg(Arg::with_name("host")
            .short('h')
            .long("host")
            .value_name("HOST")
            .help("Server hostname or IP address")
            .default_value("127.0.0.1"))
        .arg(Arg::with_name("port")
            .short('p')
            .long("port")
            .value_name("PORT")
            .help("Server port")
            .default_value("8080"))
        .arg(Arg::with_name("chunk-size")
            .short('c')
            .long("chunk-size")
            .value_name("SIZE")
            .help("Chunk size in KiB")
            .default_value("64"))
        .arg(Arg::with_name("threads")
            .short('t')
            .long("threads")
            .value_name("NUM")
            .help("Number of concurrent downloads")
            .default_value("4"))
        .arg(Arg::with_name("output")
            .short('o')
            .long("output")
            .value_name("FILE")
            .help("Save downloaded data to FILE")
            .takes_value(true))
        .arg(Arg::with_name("verify")
            .short('v')
            .long("verify")
            .value_name("HASH")
            .help("Verify SHA-256 hash of downloaded data")
            .takes_value(true))
        .arg(Arg::with_name("verbose")
            .long("verbose")
            .help("Enable verbose output with detailed error messages"))
        .get_matches();

    let host = matches.value_of("host")
        .ok_or_else(|| "Missing host argument")?;
    let port = matches.value_of("port")
        .ok_or_else(|| "Missing port argument")?
        .parse::<u16>()
        .map_err(|e| format!("Invalid port number: {}", e))?;
    let chunk_size = matches.value_of("chunk-size")
        .ok_or_else(|| "Missing chunk-size argument")?
        .parse::<usize>()
        .map_err(|e| format!("Invalid chunk size: {}", e))?
        * 1024;
    let concurrent_downloads = matches.value_of("threads")
        .ok_or_else(|| "Missing threads argument")?
        .parse::<usize>()
        .map_err(|e| format!("Invalid thread count: {}", e))?;
    let output_file = matches.value_of("output");
    let verify_hash = matches.value_of("verify");
    let verbose = matches.is_present("verbose");

    let multi_progress = MultiProgress::new();
    let total_progress = multi_progress.add(ProgressBar::new(0));
    total_progress.set_style(ProgressStyle::default_bar()
        .template("[{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta})")
        .progress_chars("#>-"));
    
    let thread_bars: Vec<_> = (0..concurrent_downloads).map(|i| {
        let pb = multi_progress.add(ProgressBar::new(chunk_size as u64));
        pb.set_style(ProgressStyle::default_bar()
            .template(&format!("Thread #{:2} [{{wide_bar:.green/white}}] {{bytes}}/{{total_bytes}}", i))
            .progress_chars("=> "));
        pb.set_position(0);
        Arc::new(Mutex::new(pb))
    }).collect();
    
    let _progress_handle = thread::spawn(move || {
        multi_progress.join().unwrap();
    });

    println!("Starting download from {}:{}", host, port);
    
    let start_time = Instant::now();
    let chunks = Arc::new(Mutex::new(Vec::<Chunk>::new()));
    let processed_chunks = Arc::new(Mutex::new(HashSet::new()));
    let total_bytes = Arc::new(Mutex::new(0_usize));
    let download_errors = Arc::new(Mutex::new(Vec::<(usize, String)>::new()));
    let total_progress = Arc::new(Mutex::new(total_progress));
    
    let mut next_chunk = 0;
    let mut eof_reached = false;
    let mut retry_count = 0;
    let max_retries = 3; // should also make configurable

    while !eof_reached && retry_count <= max_retries {
        let mut handles = vec![];

        for i in 0..concurrent_downloads {
            let chunk_id = next_chunk + i;
            let start_pos = chunk_id * chunk_size;
            let end_pos = start_pos + chunk_size;
            
            // skip processed chunks
            if processed_chunks.lock().unwrap().contains(&chunk_id) {
                continue;
            }
            
            let chunks_clone = Arc::clone(&chunks);
            let processed_clone = Arc::clone(&processed_chunks);
            let total_bytes_clone = Arc::clone(&total_bytes);
            let errors_clone = Arc::clone(&download_errors);
            let progress_bar = Arc::clone(&thread_bars[i % thread_bars.len()]);
            let total_pb = Arc::clone(&total_progress);
            let host = host.to_string();
            let verbose_flag = verbose;
            
            let handle = thread::spawn(move || {
                progress_bar.lock().unwrap().set_position(0);
                progress_bar.lock().unwrap().set_length(chunk_size as u64);
                
                let mut retry_attempts = 0;
                let max_chunk_retries = 2;
                
                loop {
                    match make_range_request_with_progress(&host, port, start_pos, end_pos, 
                                                          &progress_bar) {
                        Ok((data, headers)) => {                            
                            if headers.contains("400 Invalid range:") || data.is_empty() {
                                progress_bar.lock().unwrap().finish();
                                return Some(chunk_id); // signal EOF
                            } else {
                                progress_bar.lock().unwrap().finish();
                                
                                {
                                    let mut total = total_bytes_clone.lock().unwrap();
                                    *total += data.len();
                                    total_pb.lock().unwrap().set_position(*total as u64);
                                }
                                
                                chunks_clone.lock().unwrap().push(Chunk {
                                    id: chunk_id,
                                    data,
                                });
                                
                                processed_clone.lock().unwrap().insert(chunk_id);
                                return None;
                            }
                        }
                        Err(e) => {
                            let error_msg = format!("{}", e);
                            if verbose_flag {
                                eprintln!("Error downloading chunk {}: {}", chunk_id, error_msg);
                            }
                            
                            errors_clone.lock().unwrap().push((chunk_id, error_msg.clone()));
                            
                            retry_attempts += 1;
                            if retry_attempts <= max_chunk_retries {
                                let backoff = Duration::from_millis(50 * (1 << retry_attempts));
                                if verbose_flag {
                                    eprintln!("Retrying chunk {} after {}ms", chunk_id, backoff.as_millis());
                                }
                                thread::sleep(backoff);
                                continue;
                            } else {
                                if verbose_flag {
                                    eprintln!("Failed to download chunk {} after {} attempts", 
                                             chunk_id, retry_attempts);
                                }
                                progress_bar.lock().unwrap().finish();
                                return None;
                            }
                        }
                    }
                }
            });
            
            handles.push(handle);
        }
        
        let mut batch_eof = false;
        for handle in handles {
            match handle.join() {
                Ok(Some(_eof_chunk_id)) => {
                    batch_eof = true;
                    eof_reached = true;
                    break;
                }
                Err(e) => {
                    if verbose {
                        eprintln!("Thread panicked: {:?}", e);
                    }
                },
                _ => {}
            }
        }
        
        if !batch_eof {
            let processed = processed_chunks.lock().unwrap();
            let expected_chunks: HashSet<_> = (next_chunk..(next_chunk + concurrent_downloads)).collect();
            let missing_chunks: Vec<_> = expected_chunks.difference(&processed).collect();
            
            if !missing_chunks.is_empty() {
                if verbose {
                    eprintln!("Some chunks failed to download: {:?}", missing_chunks);
                }
                retry_count += 1;
                if retry_count > max_retries {
                    if verbose {
                        eprintln!("Max retries reached for batch starting at chunk {}. Moving to next batch.", next_chunk);
                    }
                    next_chunk += concurrent_downloads;
                    retry_count = 0;
                }
            } else {
                next_chunk += concurrent_downloads;
                retry_count = 0;
            }
        }
    }
    
    total_progress.lock().unwrap().finish_with_message("Download complete!");
    
    let mut all_chunks = chunks.lock().unwrap().clone();
    all_chunks.sort_by_key(|chunk| chunk.id);
    
    let mut all_data = Vec::new();
    for chunk in all_chunks {
        all_data.extend_from_slice(&chunk.data);
    }
    
    let mut hasher = Sha256::new();
    hasher.update(&all_data);
    let result = hasher.finalize();
    let calculated_hash = format!("{:x}", result);
    
    let total_time = start_time.elapsed().as_secs_f32();
    println!("\nDownload completed in {:.2}s", total_time);
    println!("Total size: {} bytes ({:.2} KiB)", all_data.len(), all_data.len() as f32 / 1024.0);
    println!("Average speed: {:.2} KiB/s", all_data.len() as f32 / 1024.0 / total_time);
    println!("SHA-256 hash: {}", calculated_hash);
    
    if let Some(expected_hash) = verify_hash {
        if expected_hash.to_lowercase() == calculated_hash {
            println!("Checksum verification: PASSED ✓");
        } else {
            eprintln!("Checksum verification: FAILED ✗");
            eprintln!("Expected: {}", expected_hash);
            eprintln!("Actual:   {}", calculated_hash);
            return Err("Checksum verification failed".into());
        }
    }
    
    if let Some(path) = output_file {
        println!("Saving downloaded data to '{}'", path);
        let file = File::create(Path::new(path))?;
        let mut writer = BufWriter::new(file);
        writer.write_all(&all_data)?;
        writer.flush()?;
        println!("File saved successfully");
    }
    
    let errors = download_errors.lock().unwrap();
    if !errors.is_empty() {
        let error_count = errors.len();
        eprintln!("\n{} errors occurred during download:", error_count);
        
        if verbose {
            for (chunk_id, error) in errors.iter() {
                eprintln!("Chunk {}: {}", chunk_id, error);
            }
        } else {
            eprintln!("Use --verbose for detailed error information");
        }
    }
    
    Ok(())
}

fn make_range_request_with_progress(
    host: &str, 
    port: u16, 
    start: usize, 
    end: usize,
    progress: &Arc<Mutex<ProgressBar>>,
) -> Result<(Vec<u8>, String), Box<dyn std::error::Error>> {

    let mut stream = TcpStream::connect_timeout(
        &format!("{}:{}", host, port).parse()?,
        Duration::from_secs(3)
    )?;
    
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    
    let request = format!(
        "GET / HTTP/1.1\r\n\
         Host: {}:{}\r\n\
         Range: bytes={}-{}\r\n\
         Connection: close\r\n\
         \r\n",
        host, port, start, end
    );
    
    stream.write_all(request.as_bytes())?;
    
    let mut response = Vec::with_capacity(end - start + 1024);
    let mut buffer = [0u8; 8192];
    let mut total_read = 0;
    
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => {
                response.extend_from_slice(&buffer[..n]);
                total_read += n;
                progress.lock().unwrap().set_position(total_read as u64);
            },
            Err(e) => {
                if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut {
                    if !response.is_empty() {
                        break;
                    }
                }
                return Err(Box::new(e));
            }
        }
    }
    
    if response.is_empty() {
        return Ok((Vec::new(), String::new()));
    }
    
    let mut headers_end = 0;
    for i in 0..response.len().saturating_sub(3) {
        if response[i] == b'\r' && response[i+1] == b'\n' && 
           response[i+2] == b'\r' && response[i+3] == b'\n' {
            headers_end = i + 4;
            break;
        }
    }
    
    if headers_end == 0 {
        return Ok((Vec::new(), String::new()));
    }
    
    let headers = String::from_utf8_lossy(&response[..headers_end]).to_string();
    let body = response[headers_end..].to_vec();
    
    Ok((body, headers))
}