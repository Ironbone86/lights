#![feature(proc_macro_hygiene, decl_macro)]

#[macro_use] extern crate rocket;

use std::env;
use std::thread;

use std::collections::HashMap;
use std::net::UdpSocket;
use std::sync::{Arc, Mutex};

use rocket::State;
use rocket::fairing::AdHoc;
use rocket::http::Status;

use rocket_contrib::json::Json;
use rocket_contrib::templates::Template;

use rosc::{OscPacket, OscType};

use rppal::gpio::{Gpio, OutputPin};

use serde::{Deserialize, Serialize};

use yansi::Paint;


#[derive(Clone, Copy, Serialize, Deserialize)]
struct Color {
    red: u8,
    green: u8,
    blue: u8,
}

#[derive(Clone, Serialize, Deserialize)]
struct Error {
    status: String,
    message: String,
}

type CurrentColor = Arc<Mutex<Color>>;

struct Output {
    frequency: f64,
    red_pin: OutputPin,
    green_pin: OutputPin,
    blue_pin: OutputPin,
}

type CurrentOutput = Arc<Mutex<Output>>;

#[get("/color")]
fn get_color(current: State<CurrentColor>) -> Json<Color> {
    Json(*current.lock().unwrap())
}

#[put("/color", data = "<color>")]
fn set_color(color: Json<Color>, current: State<CurrentColor>, output: State<CurrentOutput>) -> Status {
    let mut current_color = current.lock().unwrap();
    let mut current_output = output.lock().unwrap();

    current_color.red = color.red;
    current_color.green = color.green;
    current_color.blue = color.blue;

    set_output(&mut current_output, *current_color).unwrap();

    Status::NoContent
}

#[get("/")]
fn form() -> Template {
    Template::render("form", HashMap::<String, String>::new())
}

#[post("/")]
fn form_submit() -> Template {
    form()
}

#[catch(400)]
fn bad_request() -> Json<Error> {
    Json(Error {
        status: String::from("error"),
        message: String::from("Malformed request"),
    })
}

#[catch(422)]
fn unprocessable_entity() -> Json<Error> {
    Json(Error {
        status: String::from("error"),
        message: String::from("Malformed request"),
    })
}

#[catch(404)]
fn not_found() -> Json<Error> {
    Json(Error {
        status: String::from("error"),
        message: String::from("Resource not found"),
    })
}

fn osc_server(color: CurrentColor, output: CurrentOutput) {
    let address = match env::var("OSC_ADDRESS") {
        Ok(val) => val,
        Err(_err) => String::from("127.0.0.1"),
    };

    let port: u16 = match env::var("OSC_PORT") {
        Ok(val) => val.parse().unwrap(),
        Err(_err) => 1337,
    };

    let socket = UdpSocket::bind((address, port)).unwrap();

    println!("{}{} {}", Paint::masked("🎛  "), Paint::default("OSC server started on").bold(), Paint::default(socket.local_addr().unwrap()).bold().underline());

    let mut buffer = [0u8; rosc::decoder::MTU];

    loop {
        match socket.recv_from(&mut buffer) {
            Ok((size, _addr)) => {
                match rosc::decoder::decode(&buffer[..size]) {
                    Ok(packet) => {
                        match packet {
                            OscPacket::Message(msg) => {
                                match msg.addr.as_ref() {
                                    "/color" => {
                                        let mut current_color = color.lock().unwrap();
                                        let mut current_output = output.lock().unwrap();

                                        match &msg.args[..] {
                                            [OscType::Int(red), OscType::Int(green), OscType::Int(blue)] => {
                                                current_color.red = *red as u8;
                                                current_color.green = *green as u8;
                                                current_color.blue = *blue as u8;
                                            },
                                            [OscType::Float(red), OscType::Float(green), OscType::Float(blue)] => {
                                                current_color.red = *red as u8;
                                                current_color.green = *green as u8;
                                                current_color.blue = *blue as u8;
                                            },
                                            [OscType::Double(red), OscType::Double(green), OscType::Double(blue)] => {
                                                current_color.red = *red as u8;
                                                current_color.green = *green as u8;
                                                current_color.blue = *blue as u8;
                                            },
                                            [OscType::Color(color)] => {
                                                current_color.red = color.red;
                                                current_color.green = color.green;
                                                current_color.blue = color.blue;
                                            },
                                            _ => {
                                                eprintln!("Unexpected OSC /color command: {:?}", msg.args);
                                            }
                                        }

                                        set_output(&mut current_output, *current_color).unwrap();
                                    },
                                    _ => {
                                        eprintln!("Unexpected OSC Message: {}: {:?}", msg.addr, msg.args);
                                    }
                                }
                            },
                            OscPacket::Bundle(bundle) => {
                                eprintln!("Unexpected OSC Bundle: {:?}", bundle);
                            },
                        }
                    },
                    Err(err) => {
                        eprintln!("Error decoding OSC packet: {:?}", err);
                    }
                }
            },
            Err(err) => {
                eprintln!("Error receiving from socket: {}", err);
            }
        }
    }
}

fn set_output(output: &mut Output, color: Color) -> rppal::gpio::Result<()> {
    output.red_pin.set_pwm_frequency(output.frequency, color.red as f64 / 255.0)?;
    output.green_pin.set_pwm_frequency(output.frequency, color.green as f64 / 255.0)?;
    output.blue_pin.set_pwm_frequency(output.frequency, color.blue as f64 / 255.0)?;

    Ok(())
}

fn main() {
    let initial = Color { red: 242, green: 155, blue: 212 };

    let gpio = Gpio::new().unwrap();

    let mut output = Output {
        frequency: 60.0,
        red_pin: gpio.get(17).unwrap().into_output(),
        green_pin: gpio.get(27).unwrap().into_output(),
        blue_pin: gpio.get(22).unwrap().into_output(),
    };

    set_output(&mut output, initial).unwrap();

    let current_color = Arc::new(Mutex::new(initial.clone()));
    let rocket_color = Arc::clone(&current_color);
    let osc_color = Arc::clone(&current_color);

    let current_output = Arc::new(Mutex::new(output));
    let rocket_output = Arc::clone(&current_output);
    let osc_output = Arc::clone(&current_output);

    rocket::ignite()
        .mount("/", routes![get_color, set_color, form, form_submit])
        .register(catchers![bad_request, unprocessable_entity, not_found])
        .manage(rocket_color)
        .manage(rocket_output)
        .attach(Template::fairing())
        .attach(AdHoc::on_launch("OSC Server", |_rocket| {
            thread::spawn(move || {
                osc_server(osc_color, osc_output);
            });
        }))
        .launch();
}
