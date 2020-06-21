use super::bits::*;

use regex::Regex;
use std::collections::HashMap;
use std::io::BufRead;

pub struct ParseResult {
    pub pc_mapper: HashMap<u16, usize>,
    pub program: Vec<u8>
}

impl ParseResult {
    pub fn new() -> Self {
        Self {
            pc_mapper: HashMap::new(),
            program: Vec::new(),
        }
    }
}

pub fn parse_lst_file(data: &str) -> ParseResult {
    let mut result = ParseResult::new();
    let mut current_line = 1;
    let command_rgx = Regex::new(r"^([0-9A-F]{4})\s([0-9A-F]{4})").unwrap();

    for line in data.lines() {
        for cap in command_rgx.captures(line) {
            let index = u16::from_str_radix(&cap[1], 16).unwrap();
            let opcode = u16::from_str_radix(&cap[2], 16).unwrap();

            result.pc_mapper.insert(index, current_line);
            result.program.push(get_high_byte(opcode));
            result.program.push(get_low_byte(opcode));
        }

        current_line += 1;
    }

    result
}