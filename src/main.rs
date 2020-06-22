#[macro_use]
extern crate log;
extern crate hertz;
extern crate notify;

use crate::emulator::{CPU};
use std::sync::mpsc::channel;
use std::path::Path;
use std::fs::File;
use std::io::{BufReader, BufRead, Write};
use std::fs;
use regex::internal::Input;
use std::collections::VecDeque;

mod emulator;

const INPUT: &str = "gui_change.dat";
const OUTPUT: &str = "gui_set.dat";

fn input_available() -> bool {
    Path::new(INPUT).exists()
}

fn output_available() -> bool {
    !Path::new(OUTPUT).exists()
}

fn main() {
    // TODO: report all data at specific rate when running and when stopped
    simple_logger::init().unwrap();

    fs::remove_file(INPUT);
    fs::remove_file(OUTPUT);

    let (input_tx, input_rx) = channel();
    let (output_tx, output_rx) = channel();

    std::thread::spawn(|| {
        let mut cpu = CPU::new(input_rx, output_tx);
        loop { cpu.update(); }
    });

    let mut commands = VecDeque::new();
    let mut saved_string = String::new();

    loop {
        if input_available() {
            let mut input = vec![];
            loop {
                if let Ok(content) = fs::read_to_string(INPUT) {
                    fs::remove_file(INPUT).expect("Failed to delete input file");

                    for line in content.lines() {
                        input.push(String::from(line));
                    }

                    input_tx.send(input);
                    break;
                }
            }
        }

        match output_rx.try_recv() {
            Ok(data) => {
                commands.extend(data)
            },
            _ => {}
        }

        if output_available() {
            if commands.len() > 0 {
                let mut result = saved_string.clone();
                saved_string.clear();

                for i in 0..std::cmp::min(commands.len(), 1000) {
                    result += "\n";
                    result += &commands.pop_front().unwrap();
                }

               if let Err(_) = fs::write(OUTPUT, &result) {
                   saved_string = result;
               }
            }
        }
    }
}