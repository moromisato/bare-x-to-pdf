pub struct Sprm<'a> {
    pub opcode: u16,
    pub operand: &'a [u8],
}

impl Sprm<'_> {
    pub fn byte(&self) -> u8 {
        self.operand.first().copied().unwrap_or(0)
    }

    pub fn word(&self) -> u16 {
        if self.operand.len() >= 2 {
            u16::from_le_bytes([self.operand[0], self.operand[1]])
        } else {
            self.byte() as u16
        }
    }

    pub fn dword(&self) -> u32 {
        if self.operand.len() >= 4 {
            u32::from_le_bytes([self.operand[0], self.operand[1], self.operand[2], self.operand[3]])
        } else {
            self.word() as u32
        }
    }
}

pub struct SprmIter<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> SprmIter<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        SprmIter { data, pos: 0 }
    }
}

impl<'a> Iterator for SprmIter<'a> {
    type Item = Sprm<'a>;

    fn next(&mut self) -> Option<Sprm<'a>> {
        if self.pos + 2 > self.data.len() {
            return None;
        }
        let opcode = u16::from_le_bytes([self.data[self.pos], self.data[self.pos + 1]]);
        self.pos += 2;
        let spra = opcode >> 13;
        let size = match spra {
            0 | 1 => 1,
            2 | 4 | 5 => 2,
            3 => 4,
            7 => 3,
            _ => {
                if opcode == 0xD608 {
                    let len = u16::from_le_bytes([
                        *self.data.get(self.pos)?,
                        *self.data.get(self.pos + 1)?,
                    ]) as usize;
                    self.pos += 2;
                    len
                } else {
                    let len = *self.data.get(self.pos)? as usize;
                    self.pos += 1;
                    len
                }
            }
        };
        let end = (self.pos + size).min(self.data.len());
        let operand = &self.data[self.pos..end];
        self.pos = end;
        Some(Sprm { opcode, operand })
    }
}
