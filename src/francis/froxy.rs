use std::io::Cursor;

use async_std::{
    io::ReadExt,
    net::{TcpStream, ToSocketAddrs},
};
use byteorder::{BigEndian, ReadBytesExt};
use serde::Serialize;

#[derive(Copy, Clone, Serialize, Debug)]
pub struct FroxyConfig {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    pub port: u64,
}

impl FroxyConfig {
    fn new(rdr: &mut Cursor<Vec<u8>>) -> Self {
        let width = rdr.read_u16::<BigEndian>().unwrap();
        let height = rdr.read_u16::<BigEndian>().unwrap();
        let x = rdr.read_u16::<BigEndian>().unwrap();
        let y = rdr.read_u16::<BigEndian>().unwrap();
        let port = rdr.read_u64::<BigEndian>().unwrap();

        Self {
            width,
            height,
            x,
            y,
            port,
        }
    }
}

pub async fn froxy_configs<A: ToSocketAddrs + std::fmt::Display>(addr: A) -> std::io::Result<Vec<FroxyConfig>> {
    println!("Connecting to froxy {}", addr);
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await?;

    let mut rdr = Cursor::new(buf);
    let sections = rdr.read_u64::<BigEndian>().unwrap();

    Ok((0..sections).map(|_| FroxyConfig::new(&mut rdr)).collect())
}
