/*
Copyright (c) 2020 Andrew C. Young

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.
IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM,
DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR
OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE
OR OTHER DEALINGS IN THE SOFTWARE.
*/

use std::collections::{HashMap};
use std::io::{Read, Write};

pub const MAX_DICT_SIZE: usize = 4090;
pub const NOOP: u16 = 4091;
pub const FLUSH_DICTIONARY: u16 = 4093;
pub const EOF: u16 = 4094;
pub const EOS: u16 = 4095;

macro_rules! debug {
    ($debug:expr) => (if $debug { eprintln!(); });
    ($debug:expr, $fmt:expr) => (if $debug { eprintln!($fmt); });
    ($debug:expr, $fmt:expr, $($arg:tt)*) => (if $debug { eprintln!($fmt, $($arg)*); });
}

fn print_dictionary(d: &[Vec<u8>]) {
    eprintln!("Dictionary:");
    for i in 0..d.len() {
        eprintln!("\t{:04}:\t{:02X?}", i, &d[i]);
    }
}

// Returns the compression ratio as a floating point percentage.
// A value > 100 means that the file increased in size instead of decreasing.
pub fn compression_ratio(bytes_uncompressed: usize, bytes_compressed: usize) -> f64 {
    let u = bytes_uncompressed as f64;
    let c = bytes_compressed as f64;
    let ratio: f64 = (c / u) * 100.0f64;
    return ratio;
}

#[derive(Debug)]
struct Compressor {
    dict: HashMap<Vec<u8>,u16>,
    key: Vec<u8>,
    value: Option<u16>,
    write_state: Option<u16>,
    debug: bool,
    verbose: bool,
}

impl Compressor {
    fn new() -> Compressor {
        let mut c = Compressor {
            dict: HashMap::with_capacity(MAX_DICT_SIZE),
            key: Vec::with_capacity(MAX_DICT_SIZE),
            value: None,
            write_state: None,
            debug: false,
            verbose: false,
        };
        c.flush_dictionary();
        return c;
    }

    fn with_debug(debug: bool) -> Compressor {
        let mut c = Compressor::new();
        c.debug = debug;
        return c;
    }

    fn print_dictionary(&self) {
        let mut d: Vec<Vec<u8>> = Vec::with_capacity(self.dict.len());
        d.resize(self.dict.len(), vec![]);
        for (k,v) in &self.dict {
            d[*v as usize] = k.clone();
        }
        print_dictionary(&d);
    }

    fn flush_dictionary(&mut self) {
        self.dict.clear();
        // Initialize dictionary
        for n in 0..=255 {
            self.dict.insert(vec!(n), n as u16);
        }
    }

    fn encode(&mut self, value: u16, output: &mut dyn Write) -> std::io::Result<usize> {
        let mut bytes_written: usize = 0;
        self.write_state = match self.write_state {
            None => Some(value),
            Some(first) => {
                let f = first.to_le_bytes();
                let s = value.to_le_bytes();

                let out_bytes = [f[0], (f[1] << 4) | (s[0] >> 4), (s[0] << 4) | s[1]];

                if self.verbose { 
                    debug!(self.debug, "Wrote: {:?}, Value: [{:?}, {:?}]", &out_bytes, &first, &value); 
                }

                bytes_written += output.write(&out_bytes)?;

                None
            }
        };
        Ok(bytes_written)
    }

    fn compress(&mut self, input: &mut dyn Read, output: &mut dyn Write) -> std::io::Result<(usize,usize)> {
        debug!(self.debug, "Compressing");
        let mut bytes_read: usize = 0;
        let mut bytes_written: usize = 0;
        let mut bytes_read_before_flush: usize = 0;
        let mut bytes_written_before_flush: usize = 0;
        for byte in input.bytes() {
            bytes_read += 1;
            let b = byte?;
            self.key.push(b);
            let key: Vec<u8> = self.key.clone().into();
            match self.dict.get(&key) {
                // No match found
                None => {
                    if let Some(last_value) = self.value {
                        // If we had a previous value, write it out
                        bytes_written += self.encode(last_value, output)?;
                        self.value = None;
                    }

                    bytes_written += self.encode(b as u16, output)?;

                    // Add an entry to the dictionary if there is space
                    if self.dict.len() <= MAX_DICT_SIZE {
                        if self.verbose {
                            debug!(self.debug, "Adding {:?} as {:?}", &key, self.dict.len());
                        }
                        self.dict.insert(key, self.dict.len() as u16);
                    }  

                    // Clear out the key and reset it to the c
                    self.key.clear();
                },
                // Match found, look for a longer match
                Some(i) => self.value = Some(*i),
            }

            // Check compression ratio and dictionary size
            let size = self.dict.len();
            let r = bytes_read - bytes_read_before_flush;
            let w = bytes_written - bytes_written_before_flush;
            let ratio = compression_ratio(r, w) as usize;
            // TODO: Make this configurable
            if size > 4000 && ratio > 200 {
                debug!(self.debug, "Flushing Dictionary. Bytes Read: {}, Bytes Written: {}, Compression Ratio: {:3} %", r, w, ratio);
                bytes_written += self.encode(FLUSH_DICTIONARY, output)?;
                bytes_written += self.flush(output)?;
                self.flush_dictionary();
                bytes_written_before_flush = bytes_written;
                bytes_read_before_flush = bytes_read;
            }

        }
        if let Some(last_value) = self.value {
            bytes_written += self.encode(last_value, output)?;
        }

        bytes_written += self.flush(output)?;

        debug!(self.debug, "Done");
        debug!(self.debug, "------------------------------");
        return Ok((bytes_read, bytes_written));
    }

    fn flush(&mut self, output: &mut dyn Write) -> std::io::Result<usize> {
        // Flush any half written values
        match self.write_state {
            Some(_) => self.encode(NOOP, output),
            _ => Ok(0),
        }
    }

    fn end_of_file(&mut self, output: &mut dyn Write) -> std::io::Result<usize> {
        let mut bytes_written: usize = 0;
        bytes_written += self.flush(output)?;
        bytes_written += self.encode(EOF, output)?;
        bytes_written += self.encode(EOF, output)?;
        bytes_written += self.flush(output)?;
        return Ok(bytes_written);
    }

    fn end_of_stream(&mut self, output: &mut dyn Write) -> std::io::Result<usize> {
        let mut bytes_written: usize = 0;
        bytes_written += self.flush(output)?;
        bytes_written += self.encode(EOS, output)?;
        bytes_written += self.encode(EOS, output)?;
        bytes_written += self.flush(output)?;
        return Ok(bytes_written);
    }
}

#[derive(Debug)]
enum ReadState {
    Empty,
    One(u8),
    Two(u8, u8),
    EOF,
    EOS,
}

#[derive(Debug)]
struct Decompressor {
    dict: Vec<Vec<u8>>,
    key: Vec<u8>,
    read_state: ReadState,
    debug: bool,
    verbose: bool,
}

impl Decompressor {
    fn new() -> Decompressor {
        let mut d = Decompressor {
            dict: Vec::with_capacity(MAX_DICT_SIZE),
            key: Vec::with_capacity(MAX_DICT_SIZE),
            read_state: ReadState::Empty,
            debug: false,
            verbose: false,
        };
        d.flush_dictionary();
        return d;
    }

    fn with_debug(debug: bool) -> Decompressor {
        let mut d = Decompressor::new();
        d.debug = debug;
        return d;
    }

    fn print_dictionary(&self) {
        print_dictionary(&self.dict);
    }

    fn flush_dictionary(&mut self) {
        self.dict.clear();
        // Initialize dictionary
        for n in 0..=255 {
            self.dict.push(vec!(n));
        }
    }

    fn decode(&mut self, value: u16, output: &mut dyn Write) -> std::io::Result<usize> {
        let mut bytes_written = 0;

        let b: Vec<u8> = self.dict[value as usize].clone();
        bytes_written += output.write(&b)?;

        if self.verbose {
            debug!(self.debug, "Wrote: {:?}, Value: {:?}, Key: {:?}", &b, &value, &self.key);
        }

        self.key.extend(&b);

        if self.key.len() > 1 && value < 256 {
            // Single character following an extended sequence
            // This should be a new entry in the dictionary
            if self.verbose {
                debug!(self.debug, "Adding {:?} as {:?}", &self.key, self.dict.len());
            }
            self.dict.push(self.key.clone());
            self.key.clear();
        }

        return Ok(bytes_written);
    }

    fn decompress(&mut self, input: &mut dyn Read, output: &mut dyn Write) -> std::io::Result<(usize,usize)> {
        debug!(self.debug, "Decompressing");
        let mut bytes_read: usize = 0;
        let mut bytes_written: usize = 0;
        for byte in input.bytes() {
            bytes_read += 1;
            let b = byte?;
            self.read_state = match self.read_state {
                ReadState::Empty => ReadState::One(b),
                ReadState::One(f) => ReadState::Two(f, b),
                ReadState::Two(f,s) => {
                    let b = [f, s, b];
                    let first: u16 = u16::from_le_bytes([b[0], (b[1] >> 4)]);
                    let second: u16 = u16::from_le_bytes([((b[1] << 4) | b[2] >> 4), b[2] & 15]);
                    if self.verbose {         
                        debug!(self.debug, "Read: {:?}, Value: [{:?}, {:?}]", &b, &first, &second);
                    }
                    match first {
                        NOOP => ReadState::Empty,
                        EOF => ReadState::EOF,
                        EOS => ReadState::EOS,
                        _ => {
                            if first == FLUSH_DICTIONARY {
                                self.flush_dictionary();
                            } else {
                                bytes_written += self.decode(first, output)?;
                            }
                            match second {
                                NOOP => ReadState::Empty,
                                EOF => ReadState::EOF,
                                EOS => ReadState::EOS,
                                _ => {
                                    if first == FLUSH_DICTIONARY {
                                        self.flush_dictionary();
                                    } else {
                                        bytes_written += self.decode(second, output)?;
                                    }
                                    ReadState::Empty
                                },
                            }
                        },
                    }
                },
                ReadState::EOF => {
                    debug!(self.debug, "End of File");
                    debug!(self.debug, "------------------------------");
                    ReadState::EOF
                },
                ReadState::EOS => {
                    debug!(self.debug, "End of Stream");
                    debug!(self.debug, "------------------------------");
                    ReadState::EOS
                }
            };
            match self.read_state {
                ReadState::EOF => { break; },
                ReadState::EOS => { break; },
                _ => {},
            }
        }
        debug!(self.debug, "Done");
        debug!(self.debug, "------------------------------");
        return Ok((bytes_read, bytes_written));
    }

}

// An implementation of LZW 12 bit fixed width compression
pub fn compress(input: &mut dyn Read, output: &mut dyn Write, debug: bool) -> std::io::Result<(usize,usize)> {
    let mut c = Compressor::with_debug(debug);
    let (r, mut w) = c.compress(input, output)?;
    w += c.end_of_file(output)?;
    if debug {
        c.print_dictionary();
    }
    return Ok((r,w));
}

// An implementation of LZW 12 bit fixed width decompression
pub fn decompress(input: &mut dyn Read, output: &mut dyn Write, debug: bool) -> std::io::Result<(usize,usize)> {
    let mut d = Decompressor::with_debug(debug);
    let (r,w) = d.decompress(input, output)?;
    if debug {
        d.print_dictionary();
    }
    return Ok((r,w));
}

#[cfg(test)]
mod tests {
    use crate::*;

    fn uncompressed() -> Vec<u8> { 
        "Lorem ipsum dolor sit amet, consectetur adipiscing elit.
        Vestibulum ipsum nulla, pretium at leo sed, condimentum
        consectetur nisi.".as_bytes().to_vec()
    }

    fn compressed() -> Vec<u8> { 
        vec![
            76, 6, 240, 114, 6, 80, 109, 2, 0, 105, 
            7, 0, 115, 7, 80, 2, 22, 64, 111, 6, 
            192, 111, 7, 32, 32, 7, 48, 105, 7, 64, 
            32, 6, 16, 109, 6, 80, 116, 2, 192, 32, 
            6, 48, 111, 6, 224, 115, 6, 80, 99, 7, 
            64, 101, 7, 64, 117, 7, 32, 10, 22, 64, 
            3, 22, 144, 115, 6, 48, 105, 6, 224, 103, 
            2, 0, 101, 6, 192, 9, 18, 224, 10, 2, 
            0, 32, 2, 0, 27, 18, 0, 27, 21, 96, 
            101, 7, 48, 116, 6, 144, 98, 7, 80, 108, 
            7, 80, 2, 22, 144, 112, 7, 48, 117, 6, 
            208, 32, 6, 224, 117, 6, 192, 108, 6, 16, 
            44, 2, 0, 112, 7, 32, 17, 22, 144, 36, 
            18, 0, 97, 7, 64, 32, 6, 192, 101, 6, 
            240, 8, 22, 80, 100, 2, 192, 13, 22, 240, 
            110, 6, 64, 105, 6, 208, 101, 6, 224, 116, 
            7, 80, 109, 0, 160, 28, 18, 0, 55, 22, 
            48, 14, 23, 48, 101, 6, 48, 116, 6, 80, 
            53, 23, 32, 37, 22, 144, 115, 6, 144, 46, 
            15, 191, 254, 255, 239
        ]
     }

     #[test]
     fn test_simple_compress() -> std::io::Result<()> {
         let mut input: &[u8] = "          ".as_bytes();
         let expected: Vec<u8> = vec![32, 2, 0, 0, 18, 0, 1, 18, 0, 32, 15, 191, 254, 255, 239];
         let mut output: Vec<u8> = Vec::new();
         let (r, w) = compress(&mut input, &mut output, true)?;
         let ratio = compression_ratio(r, w);
         eprintln!("[Compress] Bytes Read: {}, Bytes Written: {}, Compression Ratio: {:3} %", r, w, ratio);
         assert_eq!(expected, output);
         return Ok(());
     }

    #[test]
    fn test_simple_decompress() -> std::io::Result<()> {
        let mut input: &[u8] = &vec![32, 2, 0, 0, 18, 0, 1, 18, 0, 32, 15, 191, 254, 255, 239];
        let expected: Vec<u8> = "          ".as_bytes().to_vec();
        let mut output: Vec<u8> = Vec::new();
        let (r, w) = decompress(&mut input, &mut output, true)?;
        let ratio = compression_ratio(w, r) as u64;
        eprintln!("[Decompress] Bytes Read: {}, Bytes Written: {}, Compression Ratio: {:3} %", r, w, ratio);
        assert_eq!(expected, output);
        return Ok(());
    }

    #[test]
    fn test_compress() -> std::io::Result<()> {
        let mut input: &[u8] = &uncompressed();
        let expected: Vec<u8> = compressed();
        let mut output: Vec<u8> = Vec::new();
        let (r, w) = compress(&mut input, &mut output, true)?;
        let ratio = compression_ratio(r, w) as u64;
        eprintln!("[Compress] Bytes Read: {}, Bytes Written: {}, Compression Ratio: {:3} %", r, w, ratio);
        assert_eq!(expected, output);
        return Ok(());
    }

    #[test]
    fn test_decompress() -> std::io::Result<()> {
        let mut input: &[u8] = &compressed();
        let expected: Vec<u8> = uncompressed();
        let mut output: Vec<u8> = Vec::new();
        let (r, w) = decompress(&mut input, &mut output, true)?;
        let ratio = compression_ratio(w, r) as u64;
        eprintln!("[Decompress] Bytes Read: {}, Bytes Written: {}, Compression Ratio: {:3} %", r, w, ratio);
        assert_eq!(expected, output);
        return Ok(());
    }
}
