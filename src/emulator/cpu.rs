use super::data_bus::*;
use super::instruction::*;
use super::rom_bus::*;
use super::bits::*;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Instant, Duration};
use crate::emulator::{parse_lst_file, ParseResult};
use std::fs;
use std::path::Path;
use std::panic::resume_unwind;
use std::alloc::dealloc;

pub struct CPU {
    pub cycles: usize,
    pub data_bus: DataBus,
    pub rom_bus: RomBus,
    pub input: Receiver<Vec<String>>,
    pub output: Sender<Vec<String>>,
    pub running: bool,
    program_info: ParseResult,
    last: Instant,
    now: Instant,
    jump_performed: bool,
    frame_duration: Duration,
    output_duration: Duration,
    commands: Vec<String>,
}

impl CPU {
    pub fn new(input: Receiver<Vec<String>>, output: Sender<Vec<String>>) -> Self {
        Self {
            cycles: 0,
            data_bus: DataBus::new(),
            rom_bus: RomBus::new(),
            input,
            output,
            jump_performed: false,
            commands: vec![],
            last: Instant::now(),
            now: Instant::now(),
            frame_duration: Duration::from_millis(100),
            // TODO: use this
            output_duration: Duration::from_millis(500),
            program_info: ParseResult::new(),
            running: false,
        }
    }

    pub fn reset(&mut self) {
        self.commands.clear();

        if !self.program_info.pc_mapper.is_empty() {
            self.write_command(format!("RESLINE {}", self.program_info.pc_mapper.get(&self.data_bus.get_pc()).unwrap()));
        }

        self.cycles = 0;
        self.data_bus = DataBus::new();
        self.jump_performed = false;
        self.data_bus.load_pc(0);

        self.write_command(format!("PCL {:02x}h", self.data_bus.sfr_bank.pcl));
        self.write_command(format!("PCLATH {:02x}h", self.data_bus.sfr_bank.pclath));
        self.write_command(format!("PCINTERN {:04}", self.data_bus.get_pc()));
        self.write_command(format!("WREG {:02x}h", self.data_bus.sfr_bank.w));
        self.write_command(format!("STATUS {:02x}h", self.data_bus.sfr_bank.status));
        self.write_command(format!("FSR {:02x}h", self.data_bus.sfr_bank.fsr));
        self.write_command(format!("OPTION {:02x}h", self.data_bus.sfr_bank.option));
        self.write_command(format!("TIMER0 {:02x}h", self.data_bus.sfr_bank.tmr0));
        self.write_command(String::from("STACK"));
    }

    fn write_command(&mut self, cmd: String) {
        self.commands.push(cmd);
    }

    pub fn update(&mut self) {
        self.now = Instant::now();

        let work_time = self.now - self.last;

        if work_time < self.frame_duration {
            let delta = self.frame_duration - work_time;
            std::thread::sleep(delta);
        }

        self.last = Instant::now();

        match self.input.try_recv() {
            Ok(data) => {
                for command in &data {
                    println!("{}", command);

                    if command.starts_with("C:\\") {
                        println!("Loading file: {}", command);
                        let content = fs::read_to_string(command).expect("Failed to open file");
                        let result = parse_lst_file(content.as_str());

                        self.rom_bus.load_program(&result.program, 0);
                        self.reset();

                        self.write_command(format!("SETLINE {}", result.pc_mapper.get(&0).unwrap()));
                        self.program_info = result;

                        println!("Finished loading file");
                    } else {
                        let tokens: Vec<&str> = command.split(" ").collect();
                        match tokens[0] {
                            "STEP" => self.step(),
                            "RESET" => {
                                self.reset();
                                self.write_command(format!("SETLINE {}", self.program_info.pc_mapper.get(&0).unwrap()))
                            },
                            "START" => self.running = true,
                            "STOPP" => self.running = false,
                            "XTAL" => {
                                let f_base = tokens[1].parse::<usize>().unwrap();
                                let f_mul = match tokens[2] {
                                    "kHz" => 1000,
                                    "MHz" => 1000000,
                                    _ => panic!("oopsie")
                                };

                                self.frame_duration = Duration::from_nanos(hertz::fps_to_ns_per_frame(f_base * f_mul));
                            }
                            "PORTA" => {
                                let tmp: Vec<&str> = tokens[1].split(",").collect();
                                let idx = tmp[0].parse::<usize>().unwrap();
                                let bit = tmp[1].parse::<u8>().unwrap();

                                if bit != 0 {
                                    self.set_fsr_bit(PORTA_ADDR, idx);
                                } else {
                                    self.clear_fsr_bit(PORTA_ADDR, idx);
                                }
                            }
                            "PORTB" => {
                                let tmp: Vec<&str> = tokens[1].split(",").collect();
                                let idx = tmp[0].parse::<usize>().unwrap();
                                let bit = tmp[1].parse::<u8>().unwrap();

                                if bit != 0 {
                                    self.set_fsr_bit(PORTB_ADDR, idx);
                                } else {
                                    self.clear_fsr_bit(PORTB_ADDR, idx);
                                }
                            }
                            _ => println!("Unknown input command: {}", command)
                        };
                    }
                }
            }
            _ => {}
        }

        if self.running {
            self.step();
        }

        if !self.commands.is_empty() {
            self.output.send(self.commands.clone());
            self.commands.clear();
        }
    }

    pub fn step(&mut self) {
        let old_pc = self.data_bus.get_pc();
        let result = self.rom_bus.read_instruction(old_pc);

        if let Ok(instr) = result {
            debug!("Executing {:?}", instr);
            self.execute(instr);
        } else {
            println!("{}", result.err().unwrap());
            return;
        }

        // If jump was performed one additional cycle has to be added
        self.cycles += if self.jump_performed {
            2
        } else {
            self.data_bus.inc_pc(1);
            1
        };

        self.write_command(format!("RESLINE {}", self.program_info.pc_mapper.get(&old_pc).unwrap()));
        self.write_command(format!("SETLINE {}", self.program_info.pc_mapper.get(&self.data_bus.get_pc()).unwrap()));
        self.write_command(format!("PCL {:02x}h", self.data_bus.sfr_bank.pcl));
        self.write_command(format!("PCLATH {:02x}h", self.data_bus.sfr_bank.pclath));
        self.write_command(format!("PCINTERN {:04}", self.data_bus.get_pc()));
    }

    // Getter methods
    // Flags
    fn get_carry(&self) -> bool { get_bit(self.data_bus.sfr_bank.status, C) }
    fn get_digit_carry(&self) -> bool { get_bit(self.data_bus.sfr_bank.status, DC) }
    fn get_zero(&self) -> bool { get_bit(self.data_bus.sfr_bank.status, Z) }
    // Register
    fn get_w(&self) -> u8 { self.data_bus.sfr_bank.w }
    fn get_status(&self) -> u8 { self.data_bus.sfr_bank.status }
    fn get_fsr(&mut self, mut destination: u8) -> u8 {
        self.data_bus.read_byte(if destination == INDIRECT_ADDR { self.data_bus.sfr_bank.fsr } else { destination })
    }
    fn get_fsr_bit(&mut self, destination: u8, index: usize) -> bool {
        self.data_bus.get_bit(if destination == INDIRECT_ADDR { self.data_bus.sfr_bank.fsr } else { destination }, index)
    }

    // Setter methods
    fn set_zero(&mut self, value: bool) {
        set_bit_enabled(&mut self.data_bus.sfr_bank.status, Z, value);
        self.write_command(format!("STATUSBIT {},{}", Z, value as u8));
        self.write_command(format!("STATUS {:02x}h", self.get_status()));
    }

    fn set_carry(&mut self, value: bool) {
        set_bit_enabled(&mut self.data_bus.sfr_bank.status, C, value);
        self.write_command(format!("STATUSBIT {},{}", C, value as u8));
        self.write_command(format!("STATUS {:02x}h", self.get_status()));
    }

    fn set_digit_carry(&mut self, value: bool) {
        set_bit_enabled(&mut self.data_bus.sfr_bank.status, DC, value);
        self.write_command(format!("STATUSBIT {},{}", DC, value as u8));
        self.write_command(format!("STATUS {:02x}h", self.get_status()));
    }

    fn set_w(&mut self, value: u8) {
        self.data_bus.sfr_bank.w = value;
        self.write_command(format!("WREG {:02x}h", value));
    }

    fn get_sfr_address(&mut self, destination: u8) -> u8 {
        if destination == INDIRECT_ADDR {
            self.data_bus.sfr_bank.fsr
        } else {
            destination
        }
    }

    fn set_fsr(&mut self, mut destination: u8, value: u8, dflag: bool) {
        if !dflag {
            self.set_w(value);
        } else {
            let real_addr = self.get_sfr_address(destination);
            self.data_bus.write_byte(real_addr, value);
            self.write_command(format!("FREG {},0x{:02x}", real_addr, value));
        }
    }

    fn set_fsr_bit(&mut self, destination: u8, index: usize) {
        let real_addr = self.get_sfr_address(destination);
        self.data_bus.set_bit(real_addr, index);
        let val = self.get_fsr(real_addr);
        self.write_command(format!("FREG {},0x{:02x}", real_addr, val));
    }

    fn clear_fsr_bit(&mut self, destination: u8, index: usize) {
        let real_addr = self.get_sfr_address(destination);
        self.data_bus.clear_bit(real_addr, index);
        let val = self.get_fsr(real_addr);
        self.write_command(format!("FREG {},0x{:02x}", real_addr, val));
    }

    pub fn output_stack(&mut self) {
        let mut out = String::from("STACK ");

        if self.data_bus.stack.len() > 0 {
            out += &format!("{:04}", self.data_bus.stack[0]);
        }

        for i in 1..self.data_bus.stack.len() {
            out += &format!(", {:04}", self.data_bus.stack[i]);
        }

        println!("{}", out);

        self.write_command(out);
    }

    fn push(&mut self, value: u16) {
        self.data_bus.stack.push(value);
        self.output_stack();
    }

    fn pop(&mut self) -> u16 {
        let ret = self.data_bus.stack.pop();
        self.output_stack();
        ret.unwrap()
    }

    // Checker functions
    fn check_digit_carry(&self, a: u8, b: u8) -> bool { ((a & 0xf) + (b & 0xf)) > 0xf }

    fn execute(&mut self, instruction: Instruction) {
        self.jump_performed = false;
        // TODO: Implement instructions

        match instruction {
            Instruction::Nop => {},
            Instruction::MovLw(Literal(value)) => {
                self.set_w(value);
            }
            Instruction::AndLw(Literal(value)) => {
                let val = self.get_w() & value;
                self.set_zero(val == 0);
                self.set_w(val);
            }
            Instruction::IorLw(Literal(value)) => {
                let val = self.get_w() | value;
                self.set_zero(val == 0);
                self.set_w(val);
            }
            Instruction::AddLw(Literal(value)) => {
                let (result, carry) = self.get_w().overflowing_add(value);

                self.set_zero(result == 0);
                self.set_carry(carry);
                self.set_digit_carry(self.check_digit_carry(self.get_w(), value));

                self.set_w(result);
            }
            Instruction::Goto(Address(idx)) => {
                self.data_bus.load_pc(idx);
                self.jump_performed = true
            }
            Instruction::BsF(FileRegister(destination), BitIndex(idx)) => {
                self.set_fsr_bit(destination, idx);
            }
            Instruction::MovWf(FileRegister(destination)) => {
                self.set_fsr(destination, self.get_w(), true);
            }
            Instruction::BcF(FileRegister(destination), BitIndex(idx)) => {
                self.clear_fsr_bit(destination, idx);
            }
            Instruction::SubLw(Literal(value)) => {
                let val = value.wrapping_sub(self.get_w());

                self.set_zero(val == 0);
                self.set_carry(val >= 0);
                self.set_digit_carry(self.check_digit_carry(value, self.get_w()));

                self.set_w(val);
            }
            Instruction::XorLw(Literal(value)) => {
                let val = value ^ self.get_w();
                self.set_zero(val == 0);
                self.set_w(val);
            }
            Instruction::Call(Address(idx)) => {
                self.push(self.data_bus.get_pc());
                self.data_bus.load_pc(idx - 1);
            }
            Instruction::Return => {
                let pc = self.pop();
                self.data_bus.load_pc(pc);
            }
            Instruction::RetLw(Literal(value)) => {
                self.set_w(value);
                let pc = self.pop();
                self.data_bus.load_pc(pc);
            }
            Instruction::AddWf(FileRegister(destination), DestinationFlag(dflag)) => {
                let (result, carry) = self.get_w().overflowing_add(self.get_fsr(destination));

                self.set_zero(result == 0);
                self.set_carry(carry);
                let (w, fsr) = (self.get_w(), self.get_fsr(destination));
                let dc = self.check_digit_carry(w, fsr);
                self.set_digit_carry(dc);

                self.set_fsr(destination, result, dflag);
            }
            Instruction::ClrF(FileRegister(destination)) => {
                self.set_fsr(destination, 0, true);
                self.set_zero(false);
            }
            Instruction::ComF(FileRegister(destination), DestinationFlag(dflag)) => {
                let val = !self.get_fsr(destination);
                self.set_zero(val == 0);
                self.set_fsr(destination, val, dflag);
            }
            Instruction::AndWf(FileRegister(destination), DestinationFlag(dflag)) => {
                let val = self.get_w() & self.get_fsr(destination);
                self.set_zero(val == 0);
                self.set_fsr(destination, val, dflag);
            }
            Instruction::DecF(FileRegister(destination), DestinationFlag(dflag)) => {
                let val = self.get_fsr(destination).wrapping_sub(1);
                self.set_zero(val == 0);
                self.set_fsr(destination, val, dflag);
            }
            Instruction::IncF(FileRegister(destination), DestinationFlag(dflag)) => {
                let val = self.get_fsr(destination).wrapping_add(1);
                self.set_zero(val == 0);
                self.set_fsr(destination, val, dflag);
            }
            Instruction::MovF(FileRegister(destination), DestinationFlag(dflag)) => {
                let val = self.get_fsr(destination);
                self.set_zero(val == 0);
                self.set_fsr(destination, val, dflag);
            }
            Instruction::IorWf(FileRegister(destination), DestinationFlag(dflag)) => {
                let val = self.get_fsr(destination) | self.get_w();
                self.set_zero(val == 0);
                self.set_fsr(destination, val, dflag);
            }
            Instruction::SubWf(FileRegister(destination), DestinationFlag(dflag)) => {
                let val = self.get_fsr(destination).wrapping_sub(self.get_w());
                self.set_zero(val == 0);
                let dc = self.get_fsr(destination).saturating_sub(self.get_w()) > 0xf;
                self.set_digit_carry(dc);
                let c = self.get_fsr(destination) > self.get_w();
                self.set_carry(c);
                self.set_fsr(destination, val, dflag);
            }
            Instruction::SwapWf(FileRegister(destination), DestinationFlag(dflag)) => {
                let val = self.get_fsr(destination);
                let swapped = ((val & 0xf) << 4) | ((val & 0xf0) >> 4);
                self.set_fsr(destination, swapped, dflag);
            }
            Instruction::XorWf(FileRegister(destination), DestinationFlag(dflag)) => {
                let val = self.get_w() ^ self.get_fsr(destination);
                self.set_zero(val == 0);
                self.set_fsr(destination, val, dflag);
            }
            Instruction::ClrW => {
                self.set_w(0);
                self.set_zero(true);
            }
            Instruction::BtFsc(FileRegister(destination), BitIndex(idx)) => {
                if !self.get_fsr_bit(destination, idx) {
                    self.data_bus.inc_pc(1);
                }
            }
            Instruction::BtFss(FileRegister(destination), BitIndex(idx)) => {
                if self.get_fsr_bit(destination, idx) {
                    self.data_bus.inc_pc(1);
                }
            }
            Instruction::RlF(FileRegister(destination), DestinationFlag(dflag)) => {
                let cy = self.get_carry() as u16;
                let mut val = self.get_fsr(destination) as u16;

                val = (val << 1) | cy;
                self.set_carry(val > 0xff);
                self.set_fsr(destination, val as u8, dflag);
            }
            Instruction::RrF(FileRegister(destination), DestinationFlag(dflag)) => {
                let cy = self.get_carry() as u8;
                let mut val = self.get_fsr(destination);

                let new_val = (cy << 7) | (val >> 1);
                self.set_carry(get_bit(val, 0));
                self.set_fsr(destination, new_val, dflag);
            }
            Instruction::DecFsz(FileRegister(destination), DestinationFlag(dflag)) => {
                let val = self.get_fsr(destination).wrapping_sub(1);
                if val == 0 { self.data_bus.inc_pc(1); }
                self.set_fsr(destination, val, dflag);
            }
            Instruction::IncFsz(FileRegister(destination), DestinationFlag(dflag)) => {
                let val = self.get_fsr(destination).wrapping_add(1);
                if val == 0 { self.data_bus.inc_pc(1); }
                self.set_fsr(destination, val, dflag);
            }
            _ => panic!("Unknown instruction: {:?}", instruction)
        };
    }
}
