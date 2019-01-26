extern crate midir;

extern crate gio;
extern crate gtk;

extern crate crossbeam_utils as cbu;

use std::thread;

use std::sync::mpsc::channel;
use std::sync::Mutex;
use std::sync::Arc;

use std::time::Duration;

use std::ops::Range;

use gio::prelude::*;
use gtk::prelude::*;

use midir::{MidiInput, MidiOutput};

// TODO: Work on jack coremidi backend, SysEx max 66 bytes

static APP_NAME: &str = "ldp8_config";
static DEVICE_NAME: &str = "LPD8";
static UI_SRC: &str = include_str!("lpd8_config.ui");

const BUF_LEN: usize = 1024;

const REQ_DEVICE_INFO: &[u8] = &[0xF0, 0x7E, 0x00, 0x06, 0x01, 0xF7];

const SYSEX_START: &[u8] = &[0xF0];
const SYSEX_END: &[u8] = &[0xF7];
const SYSEX_NON_REALTIME: &[u8] = &[0x7E];
const MAN_AKAI: &[u8] = &[0x47];
const MDL_LPD8: &[u8] = &[0x7F, 0x75];

enum LPD8Message {
  UploadProgram,
  SetActiveProgram,
  DownloadProgram,
  GetActiveProgram,
}

impl LPD8Message {
  fn to_pattern(&self) -> u8 {
    match self {
      UploadProgram => 0x61,
      SetActiveProgram => 0x62,
      DownloadProgram => 0x63,
      GetActiveProgram => 0x64,
    }
  }
}

#[derive(Debug, Clone)]
struct PortID(usize, String);
#[derive(Debug, Clone)]
struct DeviceIDs(PortID, PortID);

#[derive(Debug, Clone, Copy)]
struct Pad {
  note: u8,
  program_change: u8,
  control_change: u8,
  toggle: bool,
}

impl Default for Pad {
  fn default() -> Pad {
    Pad {
      note: 0,
      program_change: 0,
      control_change: 0,
      toggle: false,
    }
  }
}

#[derive(Debug, Clone, Copy)]
struct Knob {
  control_change: u8,
  low: u8,
  high: u8,
}

impl Default for Knob {
  fn default() -> Knob {
    Knob {
      control_change: 0,
      low: 0,
      high: 0,
    }
  }
}

#[derive(Debug, Clone, Copy)]
struct Program {
  channel: u8,
  pads: [Pad; 8],
  knobs: [Knob; 8],
}

impl Default for Program {
  fn default() -> Program {
    Program {
      channel: 0,
      pads: [Pad::default(); 8],
      knobs: [Knob::default(); 8],
    }
  }
}
#[derive(Debug, Clone, Copy)]
enum Response {
    Program(Program),
}

//TODO: Wrap Programs in arc mutexes to avaid poison
struct AppData {
    device_ids: Vec<DeviceIDs>,
    device_id: Arc<Mutex<Option<DeviceIDs>>>,
    programs: [Arc<Mutex<Program>>; 4],
}

impl AppData {
    fn new() -> AppData {
        AppData {
            device_ids: Vec::new(),
            device_id: Arc::new(Mutex::new(None)),
            programs: [
                Arc::new(Mutex::new(Program::default())),
                Arc::new(Mutex::new(Program::default())),
                Arc::new(Mutex::new(Program::default())),
                Arc::new(Mutex::new(Program::default())),
            ],
        }
    }
}

// TODO: May be a race, pattern matching response?
fn transact_sysex(in_name: &str, out_name: &str, request: &[u8], response_filter: &[u8], response_filter_ranges: &[Range<usize>]) -> Option<Vec<u8>> {
    let (tx, rx) = channel();
    let in_port = MidiInput::new(&APP_NAME).unwrap();
    let (in_port_id, _) = (0..in_port.port_count())
        .map(|i| (i, in_port.port_name(i).unwrap()))
        .find(|(_i, name)| name.clone() == in_name).unwrap();
    let out_port = MidiOutput::new(&APP_NAME).unwrap();
    let (out_port_id, _) = (0..out_port.port_count())
        .map(|i| (i, out_port.port_name(i).unwrap()))
        .find(|(_i, name)| name.clone() == out_name).unwrap();
        
    let _in_connection = in_port.connect(in_port_id, &APP_NAME, |_t, message, (tx, response_filter, response_filter_ranges)| {
        
        let mut data_str = String::new();
        for d in message.iter() {
            data_str += format!("{:02X?}, ", d).as_str();
        }
        println!("{}", data_str);
        let mut data_str = String::new();
        for d in response_filter.iter() {
            data_str += format!("{:02X?}, ", d).as_str();
        }
        println!("{}: {}", message.len(), data_str);
        
        if message.len() == response_filter.len() && response_filter_ranges.iter().all(|r| message[r.clone()] == response_filter[r.clone()]) {
            tx.send(message.to_vec()).unwrap();
        }
    }, (tx, response_filter.to_vec(), response_filter_ranges.to_vec())).unwrap();
    println!("{} => {}", in_name, out_name);
    thread::sleep(Duration::from_millis(1));
    let mut out_connection = out_port.connect(out_port_id, &APP_NAME).unwrap();
    out_connection.send(request).unwrap();
    if let Ok(response) = rx.recv_timeout(Duration::from_millis(2000)) { Some(response) } else { None }
}

fn push_sysex(out_name: &str, request: &[u8]) {
    let out_port = MidiOutput::new(&APP_NAME).unwrap();
    let (out_port_id, _) = (0..out_port.port_count())
        .map(|i| (i, out_port.port_name(i).unwrap()))
        .find(|(_i, name)| name.clone() == out_name).unwrap();
    let mut out_connection = out_port.connect(out_port_id, &"push_sysex").unwrap();
    
    println!("sending it '{}'", out_name);
    //thread::sleep(Duration::from_millis(300));
    
    let mut data_str = String::new();
    for d in request.iter() {
        data_str += format!("{:02X?}, ", d).as_str();
    }
    println!("{}: {}", request.len(), data_str);
    
    out_connection.send(request).unwrap();
    thread::sleep(Duration::from_millis(18));
}

fn check_info(message: &[u8]) -> bool {
    const EXPECTED: &[u8] = &[
        0xF0,
        0x7E,
        0x00, 0x06, 0x02, 0x47, 0x75, 0x00,
        0x19, 0x00, 0x00, 0x00, 0x66, 0x7F, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0xF7,
    ];
    //const EXPECTED: &[u8] = &[
    //    0xF0, 0x7E, 0x00, 0x06, 0x02, 0x47, 0x75, 0x00,
    //    0x19, 0x00, 0x00, 0x00, 0x66, 0x00, 0x00, 0x00,
    //    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    //    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    //    0x00, 0x00, 0xF7,
    //];
    //const EXPECTED: &[u8] = &[
    //    0xF0, 0x7E, 0x0A, 0x06, 0x02, 0x47, 0x75, 0x00,
    //    0x19, 0x00, 0x00, 0x00, 0x66, 0x00, 0x00, 0x00,
    //    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    //    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    //    0x00, 0x00, 0xF7,
    //];
    const TEST_RANGES: &[Range<usize>] = &[
        0..2,
        3..13,
        14..20,
    ];
    message.len() == EXPECTED.len() && TEST_RANGES.iter().all(|r| EXPECTED[r.clone()] == message[r.clone()])
}

fn download_program(device_id: &DeviceIDs, id: u8) -> Result<Program, ()> {
    let program_request = SYSEX_START.iter()
    .chain(MAN_AKAI)
    .chain(MDL_LPD8)
    .chain(&[
      0x63,
      0x00, 0x01,
      id,
    ])
    .chain(SYSEX_END)
    .cloned().collect::<Vec<_>>();
    
    const RES_EXP: &[u8] = &[
        0xF0,
        0x47,
        0x7F, 0x75,
        0x63,
        0x00, 0x3A, 0x01, 0x00, 0x20, 0x01,
        0x00, 0x00, 0x32, 0x01, 0x01, 0x00, 0x29, 0x00, 0x01, 0x00, 0x2A,
        0x03, 0x01, 0x00, 0x31, 0x27, 0x01, 0x00, 0x27, 0x00, 0x01, 0x00,
        0x33, 0x00, 0x01, 0x00, 0x39, 0x02, 0x00, 0x00, 0x01, 0x00, 0x7F,
        0x07, 0x00, 0x7F, 0x08, 0x00, 0x7F, 0x0A, 0x00, 0x7F, 0x0B, 0x00,
        0x7F, 0x02, 0x00, 0x7F, 0x04, 0x00, 0x7F, 0x54, 0x00, 0x7F, 0xF7,
    ];
    const REC_PROGRAM_TEST_RANGES: &[Range<usize>] = &[
        0..7,
    ];
    let response = transact_sysex(&(device_id.0).1, &(device_id.1).1,
        &program_request, RES_EXP, &REC_PROGRAM_TEST_RANGES);
    
    if let Some(response) = response {
      Ok(parse_program(&response).unwrap())
    } else { Err(()) }
}

fn upload_program(device_id: &DeviceIDs, id: u8, program: &Program) {
    let mut program_upload_request = SYSEX_START.iter()
    .chain(MAN_AKAI)
    .chain(MDL_LPD8)
    .chain(&[
        0x61, 0x00, 0x3A, id,
        program.channel
    ])
    .cloned().collect::<Vec<_>>();
    
    for pad in program.pads.iter() {
        program_upload_request.extend(&[pad.note, pad.program_change, pad.control_change, if pad.toggle {1} else {0}]);
    }
    for knob in program.knobs.iter() {
        program_upload_request.extend(&[knob.control_change, knob.low, knob.high]);
    }
    program_upload_request.push(0xF7);
    
    push_sysex(&(device_id.1).1, &program_upload_request);
    //transact_sysex(&(device_id.0).1, &(device_id.1).1,
    //    &program_upload_request, &[], &[]);
}

fn set_active_program_id(device_id: &DeviceIDs, p_id: u8) {
    let set_program_request = [0xF0, 0x47, 0x7F, 0x75, 0x62, 0x00, 0x01, p_id, 0xF7];
    push_sysex(&(device_id.1).1, &set_program_request);
}

fn get_active_program_id(device_id: &DeviceIDs) -> Option<u8> {
    let p_id_request = SYSEX_START.iter()
    .chain(MAN_AKAI)
    .chain(MDL_LPD8)
    .chain(&[
        0x64, 0x00, 0x00,
    ])
    .chain(SYSEX_END)
    .cloned().collect::<Vec<_>>();
    
    const P_ID_RESP: &[u8] = &[
        0xF0, 0x47, 0x7F, 0x75, 0x64, 0x00, 0x01, 0x04, 0xF7,
    ];
    const P_ID_TEST_RANGES: &[Range<usize>] = &[
        0..7,
    ];
    let response = transact_sysex(&(device_id.0).1, &(device_id.1).1,
        &p_id_request, P_ID_RESP, &P_ID_TEST_RANGES);
    if let Some(response) = response {
      Some(response[7])
    } else { None }
}

fn parse_program(message: &[u8]) -> Result<Program, ()> {
    let pads = {
        let mut i = message[9..41].chunks(4).map(|p| {
            Pad {
                note: p[0],
                program_change: p[1],
                control_change: p[2],
                toggle: p[3] == 1,
            }
        });
        [
            i.next().unwrap(), i.next().unwrap(), i.next().unwrap(), i.next().unwrap(), 
            i.next().unwrap(), i.next().unwrap(), i.next().unwrap(), i.next().unwrap(),
        ]
    };
    let knobs = {
        let mut i = message[41..65].chunks(3).map(|p| {
            Knob {
                control_change: p[0],
                low: p[1],
                high: p[2],
            }
        });
        [
            i.next().unwrap(), i.next().unwrap(), i.next().unwrap(), i.next().unwrap(), 
            i.next().unwrap(), i.next().unwrap(), i.next().unwrap(), i.next().unwrap(),
        ]
    };
    
    Ok(Program {
        channel: message[8],
        pads,
        knobs,
    })
}

fn startup(application: &gtk::Application, app_data_mutex: &Arc<Mutex<AppData>>) {
    let builder = gtk::Builder::new();
    builder.add_from_string(UI_SRC).unwrap();
    
    let window: gtk::ApplicationWindow = builder.get_object("window").expect("Couldn't get window");
    window.set_application(application);
    
    let stack: gtk::Stack = builder.get_object("prog-stack").expect("no prog stack");
    
    let device_list: gtk::ListStore = builder.get_object("device-list").expect("no midi list model");
    let device_select: gtk::ComboBox = builder.get_object("device-select").expect("dev sel not good");
    
    let midi_in = MidiInput::new(&APP_NAME).unwrap();
    let midi_out = MidiOutput::new(&APP_NAME).unwrap();
    
    println!("in: {}\nout: {}", midi_in.port_count(), midi_out.port_count());
    
    let (input_id, output_id) = {
        let input_id = (0..midi_in.port_count()).map(|i| (i, midi_in.port_name(i).unwrap_or_default()))
            //.filter(|(i, name)| true)
            .collect::<Vec<_>>();
        let output_id = (0..midi_out.port_count()).map(|i| (i, midi_out.port_name(i).unwrap_or_default()))
            //.filter(|(i, name)| true)
            .collect::<Vec<_>>();
        
        (input_id, output_id)
    };
    
    for i in input_id.iter() {
        println!("{:?}", i);
    }
    for i in output_id.iter() {
        println!("{:?}", i);
    }
    
    {
        let (tx, rx) = channel();
        let _connections = output_id.iter().map(|(i, device_name)| {
            let i = *i;
            let device_name = device_name.clone();
            let name = format!("{}_response:{}", APP_NAME, i);
            let port = MidiInput::new(&name).unwrap();
            let tx = tx.clone();
            port.connect(i, &name, move |_t, data: &[u8], _app_data_mutex| {
                if check_info(&data) {
                    tx.send((i, device_name.clone())).unwrap();
                }
                let mut data_str = String::new();
                for d in data.iter() {
                    data_str += format!("{:02X?}, ", d).as_str();
                }
                println!("{}: {}", data.len(), data_str);
            }, app_data_mutex.clone())
        }).collect::<Vec<_>>();
        
        for (i, device_name) in input_id.iter() {
            let name = format!("{}_call:{}", APP_NAME, i);
            let port = MidiOutput::new(&name).unwrap();
            let mut connection = port.connect(*i, &name).unwrap();
            connection.send(REQ_DEVICE_INFO).unwrap();
            if let Ok(id) = rx.recv_timeout(Duration::from_millis(100)) {
                println!("LPD8 is \"{:?}\"", id);
                device_list.insert_with_values(None, &[0, 1, 2, 3, 4], &[
                    &format!("{}/{}", device_name, id.1),
                    &(*i as u64), &device_name.to_string(),
                    &(id.0 as u64), &id.1]);
                let mut app_data = app_data_mutex.lock().unwrap();
                app_data.device_ids.push(DeviceIDs(PortID(*i, device_name.to_string()), PortID(id.0, id.1)));
                *app_data.device_id.lock().unwrap() = Some(app_data.device_ids.iter().next().unwrap().clone());
            }
        }
    }
    
    let output_id = output_id.iter().next().unwrap().0;
    let input_id = input_id.iter().next().unwrap().0;
    
    println!("{}", midi_in.port_name(input_id).unwrap());
    let _in_connection = midi_in.connect(input_id, &APP_NAME, |_t, data, _app_data_mutex| {
        println!("message");
        let mut data_str = String::new();
        for d in data.iter() {
            data_str += format!("{:02X?}, ", d).as_str();
        }
        println!("{}: {}", data.len(), data_str);
    }, app_data_mutex.clone()).unwrap();
    let _out_connection = midi_out.connect(output_id, &APP_NAME).unwrap();
    
    let initial_p_id = {
        let app_data = app_data_mutex.lock().unwrap();
        let device_id_mutex = app_data.device_id.clone();
        {
            let device_id = device_id_mutex.lock().unwrap().clone();
            if let Some(device_id) = device_id {
                if let Some(id) = get_active_program_id(&device_id) { id } else { 1 }
            } else { 1 }
        }
    };
    
    // let (a_send, a_rec) = channel();
    {
        let mut app_data = app_data_mutex.lock().unwrap();
        let device_id_mutex = app_data.device_id.clone();
        for (i, program_mutex) in app_data.programs.iter_mut().enumerate() {
            let id = (1 + i) as u8;
            
            let program = {
                let program = if let Some(device_id) = device_id_mutex.lock().unwrap().clone() {
                    if let Ok(program) = download_program(&device_id, id) { program } else { Program::default() }
                } else {
                    Program::default()
                };
                
                *program_mutex.lock().unwrap() = program;
                program
                //let p = download_program(app_data_mutex.clone(), id).unwrap();
                //let mut program = program_mutex.lock().unwrap();
                //*program = p;
                //p.clone()
            };
            println!("ez, {:?}", program);
            
            let layout = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            layout.set_spacing(6);
            layout.set_property_margin(6);
            
            let prog_prof = gtk::Box::new(gtk::Orientation::Vertical, 6);
            layout.add(&prog_prof);
            
            let pull_button = gtk::Button::new();
            pull_button.set_label("Fetch");
            prog_prof.add(&pull_button);
            
            {
                let program_mutex = program_mutex.clone();
                let device_id_mutex = device_id_mutex.clone();
                pull_button.connect_clicked(move |_button| {
                    if let Some(device_id) = device_id_mutex.lock().unwrap().clone() {
                        let mut program = program_mutex.lock().unwrap();
                        *program = download_program(&device_id, id).unwrap();
                    }
                    // TODO: update ui
                    //let program = download_program(app_data_mutex.clone(), id).unwrap();
                    //println!("{:?}", program);
                    //{
                    //    let mut app_data = app_data_mutex.lock().unwrap();
                    //    *app_data.programs[i].lock().unwrap() = program;
                    //}
                    println!("pull {}", i);
                });
            }
            
            let push_button = gtk::Button::new();
            push_button.set_label("Push");
            prog_prof.add(&push_button);
            {
                let program_mutex = program_mutex.clone();
                let device_id_mutex = device_id_mutex.clone();
                push_button.connect_clicked(move |_button| {
                    println!("push PROG {}", id);
                    if let Some(device_id) = device_id_mutex.lock().unwrap().clone() {
                        let program = program_mutex.lock().unwrap();
                        upload_program(&device_id, id, &program);
                    }
                    // set_program(&output, id, &*program);
                });
            }
            
            let chan_adj = gtk::Adjustment::new(f64::from(program.channel),
                0.0, 127.0,
                1.0, 0.0, 0.0);
            let chan_entry = gtk::SpinButton::new(Some(&chan_adj),
                1.0, 0);
            {
                let program_mutex = program_mutex.clone();
                chan_adj.connect_value_changed(move |adj| {
                    let mut program = program_mutex.lock().unwrap();
                    program.channel = adj.get_value() as u8;
                    println!("{}", adj.get_value());
                });
            }
            prog_prof.add(&chan_entry);
            
            let conf_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            conf_box.set_homogeneous(true);
            layout.add(&conf_box);
            
            let pad_grid = gtk::Grid::new();
            pad_grid.set_property("expand", &true).unwrap();
            pad_grid.set_column_homogeneous(true);
            pad_grid.set_row_homogeneous(true);
            pad_grid.set_column_spacing(6);
            pad_grid.set_row_spacing(6);
            conf_box.add(&pad_grid);
            
            for p_id in 0..8 {
                let pad = program.pads[p_id as usize];
                let pad_conf = gtk::Frame::new(format!("PAD {}", p_id + 1).as_str());
                let pad_lb = gtk::ListBox::new();
                pad_lb.set_property("selection-mode", &gtk::SelectionMode::None).unwrap();
                //pad_lb.set_property("activate-on-single-click", &false); ???
                
                let note_adj = gtk::Adjustment::new(f64::from(pad.note),
                    0.0, 127.0,
                    1.0, 0.0, 0.0);
                let note_entry = gtk::SpinButton::new(Some(&note_adj),
                    1.0, 0);
                {
                    let program_mutex = program_mutex.clone();
                    note_adj.connect_value_changed(move |adj| {
                        let mut program = program_mutex.lock().unwrap();
                        program.pads[p_id as usize].note = adj.get_value() as u8;
                        println!("{}", adj.get_value());
                    });
                }
                pad_lb.add(&note_entry);
                
                let prog_adj = gtk::Adjustment::new(f64::from(pad.program_change),
                    0.0, 127.0,
                    1.0, 0.0, 0.0);
                let prog_entry = gtk::SpinButton::new(Some(&prog_adj),
                    1.0, 0);
                {
                    let program_mutex = program_mutex.clone();
                    prog_adj.connect_value_changed(move |prog_adj| {
                        let mut program = program_mutex.lock().unwrap();
                        program.pads[p_id as usize].program_change = prog_adj.get_value() as u8;
                        println!("{}", prog_adj.get_value());
                    });
                }
                pad_lb.add(&prog_entry);
                
                let ctrl_adj = gtk::Adjustment::new(f64::from(pad.control_change)
                ,
                    0.0, 127.0,
                    1.0, 0.0, 0.0);
                let ctrl_entry = gtk::SpinButton::new(Some(&ctrl_adj),
                    1.0, 0);
                {
                    let program_mutex = program_mutex.clone();
                    ctrl_adj.connect_value_changed(move |adj| {
                        let mut program = program_mutex.lock().unwrap();
                        program.pads[p_id as usize].control_change = adj.get_value() as u8;
                        println!("{}", adj.get_value());
                    });
                }
                pad_lb.add(&ctrl_entry);
                
                let toggle = gtk::ToggleButton::new();
                toggle.set_label("Continuous");
                toggle.set_active(pad.toggle);
                if pad.toggle {
                    toggle.set_label("Continuous");
                } else {
                    toggle.set_label("Instant");
                }
                {
                    let program_mutex = program_mutex.clone();
                    toggle.connect_toggled(move |toggle| {
                        let mut program = program_mutex.lock().unwrap();
                        program.pads[p_id as usize].toggle = toggle.get_active();
                        if toggle.get_active() {
                            toggle.set_label("Continuous");
                        } else {
                            toggle.set_label("Instant");
                        }
                    });
                }
                pad_lb.add(&toggle);
                pad_conf.add(&pad_lb);
                pad_grid.attach(&pad_conf, p_id % 4, 1 - p_id / 4, 1, 1);
            }
            
            let knob_grid = gtk::Grid::new();
            knob_grid.set_property("expand", &true).unwrap();
            knob_grid.set_column_homogeneous(true);
            knob_grid.set_row_homogeneous(true);
            knob_grid.set_column_spacing(6);
            knob_grid.set_row_spacing(6);
            conf_box.add(&knob_grid);
            for k_id in 0..8 {
                let knob = program.knobs[k_id as usize];
                let knob_conf = gtk::Frame::new(format!("K{}", k_id + 1).as_str());
                let knob_lb = gtk::ListBox::new();
                
                let ctrl_adj = gtk::Adjustment::new(f64::from(knob.control_change),
                    0.0, 127.0,
                    1.0, 0.0, 0.0);
                let ctrl_entry = gtk::SpinButton::new(Some(&ctrl_adj),
                    1.0, 0);
                
                {
                    let program_mutex = program_mutex.clone();
                    ctrl_adj.connect_value_changed(move |adj| {
                        let mut program = program_mutex.lock().unwrap();
                        program.knobs[k_id as usize].control_change = adj.get_value() as u8;
                        println!("{}", adj.get_value());
                    });
                }
                knob_lb.add(&ctrl_entry);
                
                let low_adj = gtk::Adjustment::new(f64::from(knob.low),
                    0.0, 127.0,
                    1.0, 0.0, 0.0);
                let low_entry = gtk::SpinButton::new(Some(&low_adj),
                    1.0, 0);
                {
                    let program_mutex = program_mutex.clone();
                    low_adj.connect_value_changed(move |adj| {
                        let mut program = program_mutex.lock().unwrap();
                        program.knobs[k_id as usize].low = adj.get_value() as u8;
                        println!("{}", adj.get_value());
                    });
                }
                knob_lb.add(&low_entry);
                
                let high_adj = gtk::Adjustment::new(f64::from(knob.high),
                    0.0, 127.0,
                    1.0, 0.0, 0.0);
                let high_entry = gtk::SpinButton::new(Some(&high_adj),
                    1.0, 0);
                {
                    let program_mutex = program_mutex.clone();
                    high_adj.connect_value_changed(move |adj| {
                        let mut program = program_mutex.lock().unwrap();
                        program.knobs[k_id as usize].high = adj.get_value() as u8;
                        println!("{}", adj.get_value());
                    });
                }
                knob_lb.add(&high_entry);
                
                knob_conf.add(&knob_lb);
                knob_grid.attach(&knob_conf, k_id % 4, k_id / 4, 1, 1);
            }
            
            let name = format!("PROG {}", id);
            stack.add_titled(&layout.clone(), &id.to_string(), &name);
        }
    }
    
    
    
    //glib::timeout_add(5200, move || {
    //    println!("hi");
    //    glib::source::Continue(true)
    //});
    
    {
        let app_data_mutex = app_data_mutex.clone();
        let device_id_mutex = {
          let app_data = app_data_mutex.lock().unwrap();
          app_data.device_id.clone()
        };
        device_select.connect_changed(move |device_select| {
            // Change in_connection and out_connection
            let it = device_select.get_active_iter().unwrap();
            let device_list = device_select.get_model().unwrap();
            let _device_name = device_list.get_value(&it, 0).get::<String>();
            let in_port_name = device_list.get_value(&it, 2).get::<String>().unwrap();
            let out_port_name = device_list.get_value(&it, 4).get::<String>().unwrap();
            println!("Device select is in: {}, out: {}", in_port_name, out_port_name);
            let in_port = MidiInput::new(&APP_NAME).unwrap();
            let (in_port_id, _) = (0..in_port.port_count())
                .map(|i| (i, in_port.port_name(i).unwrap()))
                .find(|(_i, name)| name.clone() == in_port_name).unwrap();
            let out_port = MidiOutput::new(&APP_NAME).unwrap();
            let (out_port_id, _) = (0..out_port.port_count())
                .map(|i| (i, out_port.port_name(i).unwrap()))
                .find(|(_i, name)| name.clone() == out_port_name).unwrap();
            
            let d_id = DeviceIDs(PortID(in_port_id, in_port_name), PortID(out_port_id, out_port_name));
            
            *device_id_mutex.lock().unwrap() = Some(d_id);
            
            /*let in_connection = in_port.connect(in_port_id, &APP_NAME, |t, message, app_data_mutex| {
                let mut response = None;
                // TODO: This is a race
                thread::sleep(Duration::from_millis(1));
                
                const RES_EXP: &[u8] = &[
                    0xF0, 0x47, 0x7F, 0x75, 0x63, 0x00, 0x3A, 0x01, 0x00, 0x20, 0x01,
                    0x00, 0x00, 0x32, 0x01, 0x01, 0x00, 0x29, 0x00, 0x01, 0x00, 0x2A,
                    0x03, 0x01, 0x00, 0x31, 0x27, 0x01, 0x00, 0x27, 0x00, 0x01, 0x00,
                    0x33, 0x00, 0x01, 0x00, 0x39, 0x02, 0x00, 0x00, 0x01, 0x00, 0x7F,
                    0x07, 0x00, 0x7F, 0x08, 0x00, 0x7F, 0x0A, 0x00, 0x7F, 0x0B, 0x00,
                    0x7F, 0x02, 0x00, 0x7F, 0x04, 0x00, 0x7F, 0x54, 0x00, 0x7F, 0xF7,
                ];
                const REC_PROGRAM_TEST_RANGES: &[Range<usize>] = &[
                    0..7,
                ];
                
                if REC_PROGRAM_TEST_RANGES.iter().all(|r| RES_EXP[r.clone()] == message[r.clone()]) {
                    println!("He");
                    let program = parse_program(&message).unwrap();
                    response = Some(Response::Program(program));
                }
                
                
                println!("{:?}", &RES_EXP[0..7]);
                let mut data_str = String::new();
                for d in message.iter() {
                    data_str += format!("{:02X?}, ", d).as_str();
                }
                println!("{}", data_str);
                println!("{:?}", message);
                if let Some(response) = response {
                    if let Ok(app_data) = app_data_mutex.try_lock() {
                        app_data.response_tx.send(response).unwrap();
                    }
                }
                
            }, app_data_mutex.clone()).unwrap();
            let out_connection = out_port.connect(out_port_id, &APP_NAME).unwrap();
            {
                let mut app_data = app_data_mutex.lock().unwrap();
                app_data.in_connection = Some(in_connection);
                app_data.out_connection = Some(out_connection);
            }
            println!("Device select is {:?}", device_name);*/
        });
    }
    
    {
        let app_data_mutex = app_data_mutex.clone();
        stack.connect_notify("visible-child", move |stack, param| {
            // Switch device program
            let i = stack.get_visible_child_name().unwrap().parse::<u8>().unwrap();
            println!("visible child is {}", i);
            let app_data = app_data_mutex.lock().unwrap();
            if let Some(device_id) = app_data.device_id.lock().unwrap().clone() {
                set_active_program_id(&device_id, i);
            }
            println!("hey {:?} : {:?}", stack, param);
        });
    }
    
    window.show_all();
    
    device_select.set_active(0);
    stack.set_visible_child_name(&initial_p_id.to_string());
}

fn main() {
    use std::env::args;
    
    let app_data_mutex = Arc::new(Mutex::new(AppData::new()));
    
    let application = gtk::Application::new("org.kinloch.colin.lpd8_config",
        gio::ApplicationFlags::empty())
        .expect("Initialization failed...");
    
    application.connect_startup(move |application| { startup(application, &app_data_mutex) });
    application.connect_activate(|_| {});
    
    application.run(&args().collect::<Vec<_>>());
}
