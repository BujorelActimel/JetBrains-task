use std::io::{Read, Write};
use std::net::TcpStream;
use sha2::{Sha256, Digest};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // TODO - make these cusotmizable via cli flags (host, port, chunk size)
    let host = "127.0.0.1";
    let port = 8080;
    let mut start = 0;
    let mut end = 64 * 1024;

    let mut active = true;
    let mut all_data = Vec::new();

    while active {
        println!("Making test request for bytes {}-{}", start, end);
        let data = make_range_request(host, port, start, end)?;
        
        println!("Received {} bytes of data", data.len());
        println!("First few bytes: {:?}", &data[..std::cmp::min(10, data.len())]);

        all_data.extend_from_slice(&data);
        println!("Total bytes collected: {}", all_data.len());

        if data.len() == 0 {
            active = false;
        }

        start = end;
        end += 64 * 1024;
    }

    let mut hasher = Sha256::new();
    hasher.update(&all_data);
    let result = hasher.finalize();
    
    println!("SHA-256 hash of the data: {:x}", result);
    
    Ok(())
}

fn make_range_request(host: &str, port: u16, start: usize, end: usize) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut stream = TcpStream::connect(format!("{}:{}", host, port))?;
    
    let request = format!(
        "GET / HTTP/1.1\r\n\
         Host: {}:{}\r\n\
         Range: bytes={}-{}\r\n\
         Connection: close\r\n\
         \r\n",
        host, port, start, end
    );
    
    stream.write_all(request.as_bytes())?;
    
    let mut response = Vec::new();
    stream.read_to_end(&mut response)?;
    
    let mut headers_end = 0;
    for i in 0..response.len() - 3 {
        if response[i] == b'\r' && response[i+1] == b'\n' && 
           response[i+2] == b'\r' && response[i+3] == b'\n' {
            headers_end = i + 4;
            break;
        }
    }
    
    Ok(response[headers_end..].to_vec())
}