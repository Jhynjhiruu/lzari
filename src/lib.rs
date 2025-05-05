use std::cmp::Ordering;
use std::mem::size_of;

#[derive(Debug)]
pub struct LZARIContext<'a> {
    inbuf: &'a [u8],
    outbuf: Vec<u8>,
    in_buffer: u8,
    in_mask: u8,
    in_cursor: usize,
    out_buffer: u8,
    out_mask: u8,

    text_buf: Box<[u8; LZARIContext::RING_BUF_SIZE + LZARIContext::MAX_MATCH_LEN - 1]>,

    lson: Box<[usize; LZARIContext::RING_BUF_SIZE + 1]>,
    rson: Box<[usize; LZARIContext::RING_BUF_SIZE + 256 + 1]>,
    dad: Box<[usize; LZARIContext::RING_BUF_SIZE + 1]>,

    match_position: usize,

    // Arithmetic Compression
    low: usize,
    high: usize,
    value: usize,
    shifts: usize,
    char_to_sym: [usize; LZARIContext::N_CHAR],
    sym_to_char: [usize; LZARIContext::N_CHAR + 1],
    sym_freq: [usize; LZARIContext::N_CHAR + 1],
    sym_cum: [usize; LZARIContext::N_CHAR + 1],
    position_cum: Box<[usize; LZARIContext::RING_BUF_SIZE + 1]>,
}

impl<'a> LZARIContext<'a> {
    const RING_BUF_SIZE: usize = 4096;
    const MAX_MATCH_LEN: usize = 60;
    const THRESHOLD: usize = 2;
    const NIL: usize = Self::RING_BUF_SIZE;

    pub fn new(inbuf: &'a [u8]) -> Self {
        let text_buf = vec![0; Self::RING_BUF_SIZE + Self::MAX_MATCH_LEN - 1]
            .try_into()
            .unwrap();

        let lson = vec![0; Self::RING_BUF_SIZE + 1].try_into().unwrap();
        let rson = vec![0; Self::RING_BUF_SIZE + 256 + 1].try_into().unwrap();
        let dad = vec![0; Self::RING_BUF_SIZE + 1].try_into().unwrap();

        let position_cum = vec![0; Self::RING_BUF_SIZE + 1].try_into().unwrap();

        Self {
            inbuf,
            outbuf: vec![],
            in_buffer: 0,
            in_mask: 0,
            in_cursor: 0,
            out_buffer: 0,
            out_mask: 128,
            text_buf,
            lson,
            rson,
            dad,
            match_position: 0,
            low: 0,
            high: Self::Q4,
            value: 0,
            shifts: 0,
            char_to_sym: [0; Self::N_CHAR],
            sym_to_char: [0; Self::N_CHAR + 1],
            sym_freq: [0; Self::N_CHAR + 1],
            sym_cum: [0; Self::N_CHAR + 1],
            position_cum,
        }
    }

    fn put_bit(&mut self, bit: bool) {
        if bit {
            self.out_buffer |= self.out_mask;
        }
        self.out_mask >>= 1;
        if self.out_mask == 0 {
            self.outbuf.push(self.out_buffer);
            self.out_buffer = 0;
            self.out_mask = 128;
        }
    }

    fn flush_bit_buffer(&mut self) {
        for _ in 0..7 {
            self.put_bit(false);
        }
    }

    fn get_bit(&mut self) -> bool {
        self.in_mask >>= 1;
        if self.in_mask == 0 {
            self.in_buffer = if self.in_cursor < self.inbuf.len() {
                self.inbuf[self.in_cursor]
            } else {
                0xFF
            };
            self.in_cursor += 1;
            self.in_mask = 128;
        }
        self.in_buffer & self.in_mask != 0
    }

    fn init_tree(&mut self) {
        for i in Self::RING_BUF_SIZE + 1..Self::RING_BUF_SIZE + 256 + 1 {
            self.rson[i] = Self::NIL
        }

        for i in 0..Self::RING_BUF_SIZE {
            self.dad[i] = Self::NIL
        }
    }

    fn insert_node(&mut self, buf_pos: usize) -> (usize, usize) {
        let mut cmp = Ordering::Greater;
        let key = &self.text_buf[buf_pos..buf_pos + Self::MAX_MATCH_LEN];
        let mut pos = Self::RING_BUF_SIZE + 1 + usize::from(key[0]);

        self.rson[buf_pos] = Self::NIL;
        self.lson[buf_pos] = Self::NIL;

        let mut match_length = 0;

        loop {
            match cmp {
                Ordering::Greater => {
                    if self.rson[pos] != Self::NIL {
                        pos = self.rson[pos];
                    } else {
                        self.rson[pos] = buf_pos;
                        self.dad[buf_pos] = pos;
                        return (self.match_position, match_length);
                    }
                }
                _ => {
                    if self.lson[pos] != Self::NIL {
                        pos = self.lson[pos];
                    } else {
                        self.lson[pos] = buf_pos;
                        self.dad[buf_pos] = pos;
                        return (self.match_position, match_length);
                    }
                }
            }

            let idx = {
                let mut i = 1;
                while i < Self::MAX_MATCH_LEN {
                    cmp = key[i].cmp(&self.text_buf[pos + i]);
                    if cmp != Ordering::Equal {
                        break;
                    }
                    i += 1;
                }
                i
            };

            if idx > Self::THRESHOLD {
                match idx.cmp(&match_length) {
                    Ordering::Greater => {
                        self.match_position = (buf_pos - pos) & (Self::RING_BUF_SIZE - 1);
                        match_length = idx;
                        if idx >= Self::MAX_MATCH_LEN {
                            break;
                        }
                    }
                    Ordering::Equal => {
                        let temp = (buf_pos - pos) & (Self::RING_BUF_SIZE - 1);
                        if temp < self.match_position {
                            self.match_position = temp;
                        }
                    }
                    Ordering::Less => {}
                }
            }
        }
        self.dad[buf_pos] = self.dad[pos];
        self.lson[buf_pos] = self.lson[pos];
        self.rson[buf_pos] = self.rson[pos];
        self.dad[self.lson[pos]] = buf_pos;
        self.dad[self.rson[pos]] = buf_pos;
        if self.rson[self.dad[pos]] == pos {
            self.rson[self.dad[pos]] = buf_pos;
        } else {
            self.lson[self.dad[pos]] = buf_pos;
        }
        self.dad[pos] = Self::NIL;

        (self.match_position, match_length)
    }

    fn delete_node(&mut self, pos: usize) {
        if self.dad[pos] == Self::NIL {
            return;
        }
        let q = if self.rson[pos] == Self::NIL {
            self.lson[pos]
        } else if self.lson[pos] == Self::NIL {
            self.rson[pos]
        } else {
            let mut q = self.lson[pos];
            if self.rson[q] != Self::NIL {
                while self.rson[q] != Self::NIL {
                    q = self.rson[q];
                }
                self.rson[self.dad[q]] = self.lson[q];
                self.dad[self.lson[q]] = self.dad[q];
                self.lson[q] = self.lson[pos];
                self.dad[self.lson[pos]] = q;
            }
            self.rson[q] = self.rson[pos];
            self.dad[self.rson[pos]] = q;
            q
        };
        self.dad[q] = self.dad[pos];
        if self.rson[self.dad[pos]] == pos {
            self.rson[self.dad[pos]] = q;
        } else {
            self.lson[self.dad[pos]] = q;
        }
        self.dad[pos] = Self::NIL;
    }

    // Arithmetic Compression

    const M: usize = 15;

    const Q1: usize = 1 << Self::M;
    const Q2: usize = 2 * Self::Q1;
    const Q3: usize = 3 * Self::Q1;
    const Q4: usize = 4 * Self::Q1;

    const MAX_CUM: usize = Self::Q1 - 1;

    const N_CHAR: usize = 256 - Self::THRESHOLD + Self::MAX_MATCH_LEN;

    fn start_model(&mut self) {
        for sym in (1..Self::N_CHAR + 1).rev() {
            let ch = sym - 1;
            self.char_to_sym[ch] = sym;
            self.sym_to_char[sym] = ch;
            self.sym_freq[sym] = 1;
            self.sym_cum[sym - 1] = self.sym_cum[sym] + self.sym_freq[sym];
        }

        for i in (1..Self::RING_BUF_SIZE + 1).rev() {
            self.position_cum[i - 1] = self.position_cum[i] + (10000 / (i + 200));
        }
    }

    fn update_model(&mut self, sym: usize) {
        if self.sym_cum[0] >= Self::MAX_CUM {
            let mut c = 0;
            for i in (1..Self::N_CHAR + 1).rev() {
                self.sym_cum[i] = c;
                self.sym_freq[i] = (self.sym_freq[i] + 1) >> 1;
                c += self.sym_freq[i];
            }
            self.sym_cum[0] = c;
        }
        let i = {
            let mut i = sym;
            while self.sym_freq[i] == self.sym_freq[i - 1] {
                i -= 1;
            }
            i
        };
        if i < sym {
            let ch_i = self.sym_to_char[i];
            let ch_sym = self.sym_to_char[sym];
            self.sym_to_char[i] = ch_sym;
            self.sym_to_char[sym] = ch_i;
            self.char_to_sym[ch_i] = sym;
            self.char_to_sym[ch_sym] = i;
        }
        self.sym_freq[i] += 1;
        for j in (0..i).rev() {
            self.sym_cum[j] += 1;
        }
    }

    fn output(&mut self, bit: bool) {
        self.put_bit(bit);
        while self.shifts > 0 {
            self.put_bit(!bit);
            self.shifts -= 1;
        }
    }

    fn encode_char(&mut self, ch: usize) {
        let sym = self.char_to_sym[ch];
        let range = self.high - self.low;
        self.high = self.low + ((range * self.sym_cum[sym - 1]) / self.sym_cum[0]);
        self.low += (range * self.sym_cum[sym]) / self.sym_cum[0];
        loop {
            if self.high <= Self::Q2 {
                self.output(false);
            } else if self.low >= Self::Q2 {
                self.output(true);
                self.low -= Self::Q2;
                self.high -= Self::Q2;
            } else if self.low >= Self::Q1 && self.high <= Self::Q3 {
                self.shifts += 1;
                self.low -= Self::Q1;
                self.high -= Self::Q1;
            } else {
                break;
            }
            self.low += self.low;
            self.high += self.high;
        }
        self.update_model(sym);
    }

    fn encode_position(&mut self, pos: usize) {
        let range = self.high - self.low;
        self.high = self.low + ((range * self.position_cum[pos]) / self.position_cum[0]);
        self.low += (range * self.position_cum[pos + 1]) / self.position_cum[0];
        loop {
            if self.high <= Self::Q2 {
                self.output(false);
            } else if self.low >= Self::Q2 {
                self.output(true);
                self.low -= Self::Q2;
                self.high -= Self::Q2;
            } else if self.low >= Self::Q1 && self.high <= Self::Q3 {
                self.shifts += 1;
                self.low -= Self::Q1;
                self.high -= Self::Q1;
            } else {
                break;
            }
            self.low += self.low;
            self.high += self.high;
        }
    }

    fn encode_end(&mut self) {
        self.shifts += 1;
        if self.low < Self::Q1 {
            self.output(false);
        } else {
            self.output(true);
        }
        self.flush_bit_buffer();
    }

    fn binary_search_sym(&self, x: usize) -> usize {
        let mut i = 1;
        let mut j = Self::N_CHAR;
        while i < j {
            let k = (i + j) / 2;
            if self.sym_cum[k] > x {
                i = k + 1;
            } else {
                j = k;
            }
        }
        i
    }

    fn binary_search_pos(&self, x: usize) -> usize {
        let mut i = 1;
        let mut j = Self::RING_BUF_SIZE;
        while i < j {
            let k = (i + j) / 2;
            if self.position_cum[k] > x {
                i = k + 1;
            } else {
                j = k;
            }
        }
        i - 1
    }

    fn start_decode(&mut self) {
        for _ in 0..Self::M + 2 {
            self.value = (self.value << 1) + usize::from(self.get_bit())
        }
    }

    fn decode_char(&mut self) -> usize {
        let range = self.high - self.low;
        let sym =
            self.binary_search_sym((((self.value - self.low + 1) * self.sym_cum[0]) - 1) / range);
        self.high = self.low + ((range * self.sym_cum[sym - 1]) / self.sym_cum[0]);
        self.low += (range * self.sym_cum[sym]) / self.sym_cum[0];
        loop {
            if self.low >= Self::Q2 {
                self.value -= Self::Q2;
                self.low -= Self::Q2;
                self.high -= Self::Q2;
            } else if self.low >= Self::Q1 && self.high <= Self::Q3 {
                self.value -= Self::Q1;
                self.low -= Self::Q1;
                self.high -= Self::Q1;
            } else if self.high > Self::Q2 {
                break;
            }
            self.low += self.low;
            self.high += self.high;
            self.value = (self.value << 1) + usize::from(self.get_bit());
        }
        let ch = self.sym_to_char[sym];
        self.update_model(sym);
        ch
    }

    fn decode_position(&mut self) -> usize {
        let range = self.high - self.low;
        let position = self
            .binary_search_pos((((self.value - self.low + 1) * self.position_cum[0]) - 1) / range);
        self.high = self.low + ((range * self.position_cum[position]) / self.position_cum[0]);
        self.low += (range * self.position_cum[position + 1]) / self.position_cum[0];
        loop {
            if self.low >= Self::Q2 {
                self.value -= Self::Q2;
                self.low -= Self::Q2;
                self.high -= Self::Q2;
            } else if self.low >= Self::Q1 && self.high <= Self::Q3 {
                self.value -= Self::Q1;
                self.low -= Self::Q1;
                self.high -= Self::Q1;
            } else if self.high > Self::Q2 {
                break;
            }
            self.low += self.low;
            self.high += self.high;
            self.value = (self.value << 1) + usize::from(self.get_bit());
        }
        position
    }

    pub fn encode(mut self) -> Vec<u8> {
        self.outbuf.extend((self.inbuf.len() as u32).to_le_bytes());

        self.start_model();
        self.init_tree();
        let mut s = 0;
        let mut r = Self::RING_BUF_SIZE - Self::MAX_MATCH_LEN;
        for i in s..r {
            self.text_buf[i] = b' ';
        }

        let mut len = Self::MAX_MATCH_LEN.min(self.inbuf.len());
        for i in 0..len {
            self.text_buf[r + i] = self.inbuf[i];
        }

        let mut in_cursor = len;

        let mut match_length;
        let mut last_match_length;
        let mut match_position;

        for i in 1..=Self::MAX_MATCH_LEN {
            self.insert_node(r - i);
        }
        (match_position, match_length) = self.insert_node(r);

        while len > 0 {
            if match_length > len {
                match_length = len;
            }

            if match_length <= Self::THRESHOLD {
                match_length = 1;
                self.encode_char(self.text_buf[r].into());
            } else {
                self.encode_char(255 - Self::THRESHOLD + match_length);
                self.encode_position(match_position - 1);
            }
            last_match_length = match_length;
            let mut i = 0;
            while i < last_match_length.min(self.inbuf.len() - in_cursor) {
                self.delete_node(s);
                self.text_buf[s] = self.inbuf[in_cursor + i];
                if s < Self::MAX_MATCH_LEN - 1 {
                    self.text_buf[s + Self::RING_BUF_SIZE] = self.inbuf[in_cursor + i];
                }
                s = (s + 1) & (Self::RING_BUF_SIZE - 1);
                r = (r + 1) & (Self::RING_BUF_SIZE - 1);
                (match_position, match_length) = self.insert_node(r);
                i += 1;
            }
            in_cursor += i;
            while i < last_match_length {
                i += 1;
                self.delete_node(s);
                s = (s + 1) & (Self::RING_BUF_SIZE - 1);
                r = (r + 1) & (Self::RING_BUF_SIZE - 1);
                len -= 1;
                if len > 0 {
                    (match_position, match_length) = self.insert_node(r);
                }
            }
        }
        self.encode_end();

        self.outbuf
    }

    pub fn decode(mut self) -> Vec<u8> {
        let textsize = u32::from_le_bytes(self.inbuf[0..size_of::<u32>()].try_into().unwrap());

        self.in_cursor = size_of::<u32>();

        self.start_decode();
        self.start_model();
        for i in 0..Self::RING_BUF_SIZE - Self::MAX_MATCH_LEN {
            self.text_buf[i] = b' ';
        }
        let mut r = Self::RING_BUF_SIZE - Self::MAX_MATCH_LEN;

        let mut rv = vec![];

        let mut count = 0;
        while count < textsize {
            let c = self.decode_char();
            if c < 256 {
                rv.push(c as u8);
                self.text_buf[r] = c as u8;
                r = (r + 1) & (Self::RING_BUF_SIZE - 1);
                count += 1;
            } else {
                let i = (r.wrapping_sub(self.decode_position() + 1)) & (Self::RING_BUF_SIZE - 1);
                let j = c - 255 + Self::THRESHOLD;
                for k in 0..j {
                    let c = self.text_buf[(i + k) & (Self::RING_BUF_SIZE - 1)];
                    rv.push(c);
                    self.text_buf[r] = c;
                    r = (r + 1) & (Self::RING_BUF_SIZE - 1);
                    count += 1;
                }
            }
        }
        rv
    }
}
