extern crate portmidi as pm;
extern crate crossbeam_utils as cbu;

extern crate gio;
extern crate gtk;

use gio::prelude::*;
use gtk::prelude::*;

use std::time::{Duration};
use std::thread;

use std::sync::{Arc, Mutex};

const BUF_LEN: usize = 1024;

const AKAI_ID: &[u8] = &[0x47];
const LPD8_ID: &[u8] = &[0x7F, 0x75];

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

fn phex(rep: &[u8]) -> String {
  let mut this_str = String::new();
  for this_data in rep.iter() {
    this_str += format!("{:02X?}, ", this_data).as_str();
  }
  this_str
}

fn sysex_comm(input_port: &mut pm::InputPort, output_port: &pm::OutputPort, request: &[u8]) -> Vec<u8> {
  let mut req = vec![0xF0];
  req.extend_from_slice(request);
  req.extend_from_slice(&[0xF7]);
  
  let input_port = Arc::new(Mutex::new(input_port));
  let rep = Arc::new(Mutex::new(vec![]));
  {
    let rep = rep.clone();
    cbu::thread::scope(move |scope| {
      println!("its doing all programs at the same time");
      scope.spawn(move || {
        'await: loop {
          if let Ok(Some(event)) = input_port.lock().unwrap().read() {
            let m = event.message;
            let mut rep = rep.lock().unwrap();
            rep.extend(&[m.status, m.data1, m.data2, m.data3]);
          }
          let rep = rep.lock().unwrap();
          for v in rep.iter().rev() {
            if *v == 0xF7 { break 'await }
          }
        }
      });
      //thread::sleep(Duration::from_millis(1));
      output_port.write_sysex(0, &req).unwrap();
    });
  }
  let rep = rep.lock().unwrap();
  rep.clone()
}

fn identify(input_port: &mut pm::InputPort, output_port: &pm::OutputPort) {
  let rep = sysex_comm(input_port, output_port, &[
      0x7E,
      0x7F,
      0x06, 0x01
    ]);
  println!("identify: {}", phex(&rep));
  let dev_id = &rep[5..9];
  let ver = &rep[9..13];
  println!("device: {} version: {}", phex(&dev_id), phex(&ver));
  const IDENT_RESP_HEADER: [u8; 5] = [
    0xF0, // SysEx
    0x7E, // Non-Realtime
    0x0A, // channel (0x00-0x7F)
    0x06, // General Info
    0x02 // Identity Reply
    ];
  assert!(rep[0..5].iter().zip(IDENT_RESP_HEADER.iter()).all(|(a, b)| a == b));
}

fn activate_program(output_port: &pm::OutputPort, prog: u8) {
  println!("Activating program {}", prog);
  let push = vec![
    0xF0,
    0x47,
    0x7F, 0x75,
    0x62, 0x00, 0x01, prog,
    0xF7,
  ];
  output_port.write_sysex(0, &push).unwrap();
}

fn get_active_program_id(input_port: &mut pm::InputPort, output_port: &pm::OutputPort) -> u8 {
  let rep = sysex_comm(input_port, output_port, &[
    0x47,
    0x7F, 0x75,
    0x64, 0x00, 0x00,
    ]);
  let prog = *rep.get(7).unwrap();
  println!("Current program is {}", prog);
  prog
}

fn set_program(output_port: &pm::OutputPort, prog: u8, program: &Program) {
  let mut push = vec![
    0xF0,
    0x47,
    0x7F, 0x75,
    0x61, 0x00, 0x3A, prog,
    program.channel];
  for pad in program.pads.iter() {
    push.extend(&[pad.note, pad.program_change, pad.control_change, if pad.toggle {1} else {0}]);
  }
  for knob in program.knobs.iter() {
    push.extend(&[knob.control_change, knob.low, knob.high]);
  }
  push.push(0xF7);
  println!("Pushing Program {}", prog);
  output_port.write_sysex(0, &push).unwrap();
}

fn get_program(input_port: &mut pm::InputPort, output_port: &pm::OutputPort, prog: u8) -> Program {
  //let mut program = vec![];
  /*output_port.write_sysex(0, &prog_request(prog)).unwrap();
  loop {
    if let Ok(Some(event)) = input_port.read() {
      let m = event.message;
      program.extend(&[m.status, m.data1, m.data2, m.data3]);
    }
    if program.len() > 66 { break }
  }*/
  let program = sysex_comm(input_port, output_port, &prog_request(prog));//get_program_bytes(input_port, &output_port, prog);
  
  let mut this_str = String::new();
  for this_data in program.iter() {
    this_str += format!("{:02X?}, ", this_data).as_str();
  }
  println!("prep : {}", this_str);
  let pads = {
    let mut i = program.get(9..40).unwrap().chunks(4).map(|p| {
      Pad {
        note: *p.get(0).unwrap(),
        program_change: *p.get(1).unwrap(),
        control_change: *p.get(2).unwrap(),
        toggle: *p.get(0).unwrap() == 1,
      }
    });
    [
      i.next().unwrap(), i.next().unwrap(), i.next().unwrap(), i.next().unwrap(), 
      i.next().unwrap(), i.next().unwrap(), i.next().unwrap(), i.next().unwrap(),
    ]
  };
  let knobs = {
    let mut i = program.get(41..65).unwrap().chunks(3).map(|p| {
      Knob {
        control_change: *p.get(0).unwrap(),
        low: *p.get(1).unwrap(),
        high: *p.get(2).unwrap(),
      }
    });
    [
      i.next().unwrap(), i.next().unwrap(), i.next().unwrap(), i.next().unwrap(), 
      i.next().unwrap(), i.next().unwrap(), i.next().unwrap(), i.next().unwrap(),
    ]
  };
  
  return Program {
    channel: *program.get(8).unwrap(),
    pads: pads,
    knobs: knobs,
  }
}

fn query_device_info(input_port: &mut pm::InputPort, output_port: &pm::OutputPort) -> Option<[u8; 4]> {
  let sysex = sysex_comm(input_port, output_port, &[0x7E, 0x00, 0x06, 0x01]);
  let d = sysex.get(10..14).unwrap().clone();
  Some([d[0], d[1], d[2], d[3]])
}

fn prog_request(prog: u8) -> Vec<u8> {
  let mut message = vec![];
  message.extend_from_slice(&AKAI_ID);
  message.extend_from_slice(&LPD8_ID);
  message.extend_from_slice(&[0x63, 0x00, 0x01, prog]);
  let mut b_str = String::new();
  for m in message.iter() {
    b_str += &format!("{:02X?}, ", m);
  }
  println!("preq : {}", b_str);
  message
}

fn startup(application: &gtk::Application) {
  let mut programs = [
    Arc::new(Mutex::new(Program::default())),
    Arc::new(Mutex::new(Program::default())),
    Arc::new(Mutex::new(Program::default())),
    Arc::new(Mutex::new(Program::default())),
  ];
  
  let ui_src = include_str!("lpd8_config.ui");
  let builder = gtk::Builder::new();
  builder.add_from_string(ui_src).unwrap();
  let window: gtk::ApplicationWindow = builder.get_object("window").expect("Couldn't get window");
  window.set_application(application);
  
  let midi_list_model: gtk::ListStore = builder.get_object("midi-list").expect("no midi list model");
  let midi_combobox: gtk::ComboBox = builder.get_object("midi-combobox").expect("no midi combobox");
  
  
  let context_mutex = Arc::new(Mutex::new(pm::PortMidi::new().unwrap()));
  let (input_mutex, output_mutex) = {
    let context = context_mutex.lock().unwrap();
    let devices = context.devices().unwrap();
    let lpd8_devices = devices.iter().filter(|d| d.name() == "LPD8");
    let mut in_ports = lpd8_devices.clone().filter(|d| d.is_input())
      .filter_map(|dev| context.input_port(dev.clone(), BUF_LEN).ok());
    let mut out_ports = lpd8_devices.clone().filter(|d| d.is_output())
      .filter_map(|dev| context.output_port(dev.clone(), BUF_LEN).ok());
    
    /*for mut o in out_ports.clone() {
      for mut i in in_ports.clone() {
        let d = query_device_info(&i, &o).unwrap();
        let mut d_str = String::new();
        for dd in d.iter() {
          d_str += &format!("{:02X?}, ", dd);
        }
        println!("{}", d_str);
        let d = get_program(&mut i, &o, 1);
        println!("{:?}", d);
      }
    }*/
    (Arc::new(Mutex::new(in_ports.next().unwrap())), Arc::new(Mutex::new(out_ports.next().unwrap())))
  };
  
  // TODO:
  // Filter by name == 'LPD8'
  // listen to all inputs
  // Iterate;
  //   out request
  //   in reply
  //   pair (out, in)
  // list
  
  let init_program_id = {
    let mut input = input_mutex.lock().unwrap();
    let output = output_mutex.lock().unwrap();
    identify(&mut input, &output);
    get_active_program_id(&mut input, &output)
  };
  println!("Initial ID is {}", init_program_id);
  
  let dev_info = {
    let mut input = input_mutex.lock().unwrap();
    let output = output_mutex.lock().unwrap();
    query_device_info(&mut input, &output).unwrap()
  };
  println!("Device version: {:?}", dev_info);
  
  let stack: gtk::Stack = builder.get_object("prog-stack").expect("no prog stack");
  for (i, prog_mutex) in programs.iter_mut().enumerate() {
    let mut program = prog_mutex.lock().unwrap();
    let id = i as u8 + 1;
    let mut input = input_mutex.lock().unwrap();
    let mut output = output_mutex.lock().unwrap();
    *program = get_program(&mut input, &output, id);
    //builder.add_objects_from_string(ui_layout_src, &["layout"]);
    //let layout: gtk::Widget = builder.get_object("layout").unwrap();
    let layout = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    layout.set_spacing(6);
    layout.set_property_margin(6);
    
    let prog_prof = gtk::Box::new(gtk::Orientation::Vertical, 6);
    layout.add(&prog_prof);
    
    let pull_button = gtk::Button::new();
    pull_button.set_label("Fetch");
    prog_prof.add(&pull_button);
    {
      let context_mutex = context_mutex.clone();
      let input_mutex = input_mutex.clone();
      let output_mutex = output_mutex.clone();
      let mut prog_mutex = prog_mutex.clone();
      pull_button.connect_clicked(move |_button| {
        // TODO Fix portmidi-rs to relate lifetime of port to context
        let context = context_mutex.lock().unwrap();
        let mut input = input_mutex.lock().unwrap();
        let output = output_mutex.lock().unwrap();
        let mut program = prog_mutex.lock().unwrap();
        println!("pull the val");
        *program = get_program(&mut input, &output, id);
        println!("u");
      });
    }
    
    let push_button = gtk::Button::new();
    push_button.set_label("Push");
    prog_prof.add(&push_button);
    {
      let context_mutex = context_mutex.clone();
      let output_mutex = output_mutex.clone();
      let mut prog_mutex = prog_mutex.clone();
      push_button.connect_clicked(move |_button| {
        let context = context_mutex.lock().unwrap();
        let output = output_mutex.lock().unwrap();
        let program = prog_mutex.lock().unwrap();
        println!("pull the val");
        set_program(&output, id, &*program);
        println!("u");
      });
    }
    
    let chan_adj = gtk::Adjustment::new(program.channel as f64,
      0.0, 127.0,
      1.0, 0.0, 0.0);
    let chan_entry = gtk::SpinButton::new(Some(&chan_adj),
      1.0, 1);
    {
      let mut prog_mutex = prog_mutex.clone();
      chan_adj.connect_value_changed(move |adj| {
        let mut program = prog_mutex.lock().unwrap();
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
      
      let note_adj = gtk::Adjustment::new(pad.note as f64,
        0.0, 127.0,
        1.0, 0.0, 0.0);
      let note_entry = gtk::SpinButton::new(Some(&note_adj),
        1.0, 1);
      {
        let mut prog_mutex = prog_mutex.clone();
        note_adj.connect_value_changed(move |note_adj| {
          let mut program = prog_mutex.lock().unwrap();
          program.pads[p_id as usize].note = note_adj.get_value() as u8;
          println!("{}", note_adj.get_value());
          
        });
      }
      pad_lb.add(&note_entry);
      
      let prog_adj = gtk::Adjustment::new(pad.program_change as f64,
        0.0, 127.0,
        1.0, 0.0, 0.0);
      let prog_entry = gtk::SpinButton::new(Some(&prog_adj),
        1.0, 1);
      {
        let mut prog_mutex = prog_mutex.clone();
        prog_adj.connect_value_changed(move |prog_adj| {
          let mut program = prog_mutex.lock().unwrap();
          program.pads[p_id as usize].program_change = prog_adj.get_value() as u8;
          println!("{}", prog_adj.get_value());
          
        });
      }
      pad_lb.add(&prog_entry);
      
      let ctrl_adj = gtk::Adjustment::new(pad.control_change as f64,
        0.0, 127.0,
        1.0, 0.0, 0.0);
      let ctrl_entry = gtk::SpinButton::new(Some(&ctrl_adj),
        1.0, 1);
      {
        let mut prog_mutex = prog_mutex.clone();
        ctrl_adj.connect_value_changed(move |ctrl_adj| {
          let mut program = prog_mutex.lock().unwrap();
          program.pads[p_id as usize].control_change = ctrl_adj.get_value() as u8;
          println!("{}", ctrl_adj.get_value());
          
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
        let mut prog_mutex = prog_mutex.clone();
        toggle.connect_toggled(move |toggle| {
          let mut program = prog_mutex.lock().unwrap();
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
      
      let ctrl_adj = gtk::Adjustment::new(knob.control_change as f64,
        0.0, 127.0,
        1.0, 0.0, 0.0);
      let ctrl_entry = gtk::SpinButton::new(Some(&ctrl_adj),
        1.0, 1);
      {
        let mut prog_mutex = prog_mutex.clone();
        ctrl_adj.connect_value_changed(move |adj| {
          let mut program = prog_mutex.lock().unwrap();
          program.knobs[k_id as usize].control_change = adj.get_value() as u8;
          println!("{}", adj.get_value());
          
        });
      }
      knob_lb.add(&ctrl_entry);
      
      let low_adj = gtk::Adjustment::new(knob.low as f64,
        0.0, 127.0,
        1.0, 0.0, 0.0);
      let low_entry = gtk::SpinButton::new(Some(&low_adj),
        1.0, 1);
      {
        let mut prog_mutex = prog_mutex.clone();
        low_adj.connect_value_changed(move |adj| {
          let mut program = prog_mutex.lock().unwrap();
          program.knobs[k_id as usize].low = adj.get_value() as u8;
          println!("{}", adj.get_value());
          
        });
      }
      knob_lb.add(&low_entry);
      
      let high_adj = gtk::Adjustment::new(knob.high as f64,
        0.0, 127.0,
        1.0, 0.0, 0.0);
      let high_entry = gtk::SpinButton::new(Some(&high_adj),
        1.0, 1);
      {
        let mut prog_mutex = prog_mutex.clone();
        high_adj.connect_value_changed(move |adj| {
          let mut program = prog_mutex.lock().unwrap();
          program.knobs[k_id as usize].high = adj.get_value() as u8;
          println!("{}", adj.get_value());
          
        });
      }
      knob_lb.add(&high_entry);
      
      knob_conf.add(&knob_lb);
      knob_grid.attach(&knob_conf, k_id % 4, k_id / 4, 1, 1);
    }
    
    let name = format!("PROG {}", i + 1);
    stack.add_titled(&layout.clone(), &id.to_string(), &name);
  }
  
  {
    let output_mutex = output_mutex.clone();
    stack.connect("notify::visible-child", true, move |v| {
      let ref stack = v[0].get::<gtk::Stack>().unwrap();
      println!("{:?}", stack.get_visible_child_name());
      let i = stack.get_visible_child_name().unwrap().parse::<u8>().unwrap();
      println!("visible child is {}", i);
      let o = output_mutex.lock().unwrap();
      activate_program(&o, i);
      println!("hey {} {:?}", i, stack);
      None
    }).unwrap();
  }
  
  window.show_all();
  
  {
    println!("Setting visible child to {}", &init_program_id.to_string());
    stack.set_visible_child_name(&init_program_id.to_string());
  }
}

fn main() {
  use std::env::args;
  
  let application = gtk::Application::new("org.kinloch.colin.lpd8_config",
      gio::ApplicationFlags::empty())
    .expect("Initialization failed...");
  
  application.connect_startup(startup);
  application.connect_activate(|_| {});
  
  application.run(&args().collect::<Vec<_>>());
}
