use std::io::{Read, Write};
use std::net::TcpStream;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let host = "127.0.0.1";
    let port = 8080;
    let start = 0;
    let end = 64 * 1024;
    
    println!("Making test request for bytes {}-{}", start, end);
    let data = make_range_request(host, port, start, end)?;
    
    println!("Received {} bytes of data", data.len());
    println!("First few bytes: {:?}", &data[..std::cmp::min(10, data.len())]);
    
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