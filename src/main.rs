mod opencbm;

use svg2gcode::{
    Machine,
    SupportedFunctionality, svg2program, ConversionConfig, ConversionOptions,
};
use svgtypes::{
    Length,
    LengthUnit
};
use g_code::{
    parse::snippet_parser,
    emit::{Token, Value},
};
use std::{
    fs::read_to_string,
    path::PathBuf
};
use roxmltree::Document;
use clap::Parser;

const PLOTTER_DEVICE: u8 = 6;
const PLOTTER_SA_XY: u8 = 1;
const PLOTTER_SA_RESET: u8 = 7;

#[derive(Default)]
struct MoveOptions {
    x: Option<u32>,
    y: Option<u32>,
}

#[allow(dead_code)]
enum Commands {
    Move(MoveOptions),
    Draw(MoveOptions),
    Reset,
}

impl Commands {
    pub fn new_move() -> Self {
        Self::Move(MoveOptions { ..Default::default() })
    }

    pub fn new_draw() -> Self {
        Self::Draw(MoveOptions { ..Default::default() })
    }

    pub fn is_ready(&self) -> bool {
        match self {
            Self::Move(opts) => opts.x.is_some() && opts.y.is_some(),
            Self::Draw(opts) => opts.x.is_some() && opts.y.is_some(),
            _ => true,
        }
    }

    pub fn set_x(&mut self, x: u32) {
        match self {
            Self::Move(ref mut opts) => opts.x = Some(x),
            Self::Draw(ref mut opts) => opts.x = Some(x),
            _ => {},
        }
    }

    pub fn set_y(&mut self, y: u32) {
        match self {
            Self::Move(ref mut opts) => opts.y = Some(y),
            Self::Draw(ref mut opts) => opts.y = Some(y),
            _ => {},
        }
    }
}

struct Plotter {
    driver: isize,
    args: Args,
}

impl Plotter {
    pub fn new(args: Args) -> Self {
        unsafe {
            let mut driver: isize = 0;
            let res = opencbm::cbm_driver_open_ex(
                &mut driver,
                args.adapter.clone().unwrap_or(String::new()).as_mut_str() as *mut _ as *mut i8
            );

            if res != 0 {
                panic!("failed to open adapter with error: {}", res);
            }

            Self {
                driver,
                args,
            }
        }
    }

    pub fn plot(mut self) {
        let contents = read_to_string(self.args.file.clone()).expect("failed to read file");
        let doc = Document::parse(contents.as_str()).expect("failed to parse file");

        let tool_on = Some("Z0").map(snippet_parser).transpose().unwrap(); 
        let tool_off = Some("Z1").map(snippet_parser).transpose().unwrap(); 

        let machine = Machine::new(
            SupportedFunctionality { circular_interpolation: false},
            tool_on,
            tool_off,
            None,
            None
        );

        println!("Machine started, converting SVG");
        let gcode = svg2program(
            &doc, 
            &ConversionConfig {
                tolerance: 0.1,
                feedrate: 55.0,
                dpi: 200.0,
                origin: [None, None],
            },
            ConversionOptions {
                dimensions: [
                    Some(Length::new(self.args.width as f64, LengthUnit::Mm)), 
                    Some(Length::new(self.args.height as f64, LengthUnit::Mm))
                ],
            },
            machine,
        );

        let mut command = Commands::new_move();

        for token in gcode {
            if let Token::Field(value) = token {
                let letter_type: &str = value.letters.as_ref();
                match letter_type {
                    "X" => {
                        if let Value::Float(value) = value.value {
                            command.set_x(value as u32);
                        }
                    },
                    "Y" => {
                        if let Value::Float(value) = value.value {
                            command.set_y(value as u32);
                        }
                    },
                    "Z" => {
                        if let Value::Integer(value) = value.value {
                            match value {
                                0 => command = Commands::new_draw(),
                                1 => command = Commands::new_move(),
                                _ => panic!("unrecognised tool command!"),
                            }
                        }
                    }
                    _ => {},
                }
            }

            if command.is_ready() {
                match command {
                    Commands::Reset => unsafe {
                        opencbm::cbm_listen(self.driver, PLOTTER_DEVICE, PLOTTER_SA_RESET);
                        opencbm::cbm_raw_write(self.driver, std::ptr::null(), 0);
                        opencbm::cbm_unlisten(self.driver);
                    },
                    Commands::Move(ref opts) => unsafe {
                        let command = std::ffi::CString::new(
                            format!("M,{},{}\n", opts.x.unwrap(), opts.y.unwrap())
                        ).unwrap()
                         .into_bytes_with_nul();

                        self.write(PLOTTER_DEVICE, PLOTTER_SA_XY, command.as_slice());
                    },
                    Commands::Draw(ref opts) => unsafe {
                        let command = std::ffi::CString::new(
                            format!("D,{},{}", opts.x.unwrap(), opts.y.unwrap())
                        ).unwrap()
                         .into_bytes_with_nul();
                        
                        self.write(PLOTTER_DEVICE, PLOTTER_SA_XY, command.as_slice());
                    }
                }
            }
        }
    }

    unsafe fn write(&mut self, addr: u8, sec_addr: u8, data: &[u8]) {
        let open_err = opencbm::cbm_open(self.driver, addr, sec_addr, std::ptr::null(), 0);

        if open_err != 0 {
            panic!("failed to open address {}:{}", addr, sec_addr);
        }
            
        opencbm::cbm_listen(self.driver, addr, sec_addr);
        opencbm::cbm_raw_write(self.driver, data.as_ptr() as *mut _, data.len());
        opencbm::cbm_unlisten(self.driver);
        opencbm::cbm_close(self.driver, addr, sec_addr);
    }
}

impl Drop for Plotter {
    fn drop(&mut self) {
        unsafe {
            opencbm::cbm_driver_close(self.driver);
        }
    }
}

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(long, help = "Path to the SVG file to plot")]
    pub file: PathBuf,
    
    #[arg(long, help = "OpenCBM adapter name")]
    pub adapter: Option<String>,

    #[arg(long, help = "Height in mm (Max 997)")]
    pub height: u32,

    #[arg(long, help = "Width in mm (Max 447")]
    pub width: u32,
}

fn main() {
    let args = Args::parse();

    if args.height > 997 || args.width > 447 {
        panic!("Invalid width/height");
    }

    let plotter = Plotter::new(args);
    plotter.plot();
}
