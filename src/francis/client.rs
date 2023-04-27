use std::io::Cursor;

use async_std::{
    io::{self, ReadExt, WriteExt},
    net::{TcpStream, ToSocketAddrs},
};
use byteorder::WriteBytesExt;
use byteorder::{BigEndian, ReadBytesExt};
use wgpu::util::align_to;

pub struct Francis {
    width: u16,
    height: u16,
    stream: TcpStream,
    buffer: Option<Vec<u8>>,
}

const ITEMS: usize = 5000;
impl Francis {
    pub async fn new<A: ToSocketAddrs>(
        addr: A,
        w: Option<u16>,
        h: Option<u16>,
    ) -> io::Result<Self> {
        let mut stream = TcpStream::connect(addr).await?;

        let mut buf = [0u8; 4];
        stream.read_exact(&mut buf).await?;
        let mut rdr = Cursor::new(buf);
        let mut width = rdr.read_u16::<BigEndian>().unwrap();
        let mut height = rdr.read_u16::<BigEndian>().unwrap();

        if let Some(w) = w {
            width = w;
        }
        if let Some(h) = h {
            height = h;
        }

        let width = align_to(width, 64);
        Ok(Self {
            width,
            height,
            stream,
            buffer: None,
        })
    }

    pub fn width(&self) -> u32 {
        self.width as u32
    }

    pub fn height(&self) -> u32 {
        self.height as u32
    }

    pub async fn write(&mut self, buf: Vec<u8>, bytes_per_pixel: usize) -> io::Result<()> {
        debug_assert_eq!(
            buf.len(),
            bytes_per_pixel * self.width as usize * self.height as usize
        );

        let mut cursor = Cursor::new([0; 7 * ITEMS]);
        let mut i = 0;

        for x in 0..self.width {
            for y in 0..self.height {
                let index = (y as usize * self.width as usize + x as usize) * bytes_per_pixel;

                let b = buf[index + 0];
                let g = buf[index + 1];
                let r = buf[index + 2];

                if let Some(old) = &self.buffer {
                    let or = old[index + 0];
                    let og = old[index + 1];
                    let ob = old[index + 2];
                    if or == r && og == g && ob == b {
                        continue;
                    }
                }

                cursor.write_u16::<BigEndian>(x).unwrap();
                cursor.write_u16::<BigEndian>(y).unwrap();

                cursor.write_u8(r).unwrap();
                cursor.write_u8(g).unwrap();
                cursor.write_u8(b).unwrap();
                i += 1;

                if i == ITEMS {
                    self.stream.write_all(cursor.get_ref()).await?;
                    cursor.set_position(0);
                    i = 0;
                }
            }
        }

        self.stream.write_all(cursor.get_ref()).await?;
        self.stream.flush().await?;
        cursor.set_position(0);

        self.buffer = Some(buf);

        Ok(())
    }
}
