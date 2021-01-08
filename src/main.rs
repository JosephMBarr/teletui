mod event;
use crossbeam::thread;
use event::{Event, Events};
use rtdlib::types::*;
use rtdlib::Tdlib;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Error, ErrorKind};
use std::sync::{Arc, Mutex};
use std::vec;
use termion::{event::Key, input::MouseTerminal, raw::IntoRawMode, screen::AlternateScreen};
use tui::{
    backend::TermionBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::Text,
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Terminal,
};

const TIMEOUT: f64 = 60.0;
const DEBUG_LEVEL: i64 = 0;
const NUM_BLOCKS: i32 = 3;

/*
enum InputMode {
    Normal,
    Insert,
}
*/
struct Chats {
    chat_vec: Arc<Mutex<Vec<Chat>>>,
    name: &'static str,
    selected_index: i32,
}
trait TBlock {
    fn new(name: &'static str) -> Self;

    fn scroll_up(&mut self) {}
    fn scroll_down(&mut self) {}
    fn get_len(&self) -> Result<i32, io::Error> {
        Ok(-1)
    }
}

impl TBlock for Chats {
    fn new(name: &'static str) -> Chats {
        Chats {
            chat_vec: Arc::new(Mutex::new(Vec::new())),
            name: name,
            selected_index: 0,
        }
    }
    fn get_len(&self) -> Result<i32, io::Error> {
        let cur_size: usize = self.chat_vec.lock().unwrap().len();
        if cur_size > u32::MAX as usize {
            let e = Error::new(ErrorKind::Other, "too many chats!");
            Err(e)
        } else {
            Ok(cur_size as i32)
        }
    }
    fn scroll_up(&mut self) {
        self.selected_index = (self.selected_index - 1) % self.get_len().unwrap();
    }
    fn scroll_down(&mut self) {
        self.selected_index = (self.selected_index + 1) % self.get_len().unwrap();
    }
}

fn main() -> io::Result<()> {
    let tdlib = Tdlib::new();
    let set_verbosity_level = SetLogVerbosityLevel::builder()
        .new_verbosity_level(DEBUG_LEVEL)
        .build();
    tdlib.send(&set_verbosity_level.to_json().unwrap());

    let (api_id, api_hash, phone_number) = read_info().unwrap();
    let mut input_str = String::new();
    //Main listening loop
    thread::scope(|s| {
        let chat_list = Chats::new("Chats");
        let rec_chat_vec = chat_list.chat_vec.clone();
        let _rec_thread = s.spawn(move |_| {
            loop {
                match tdlib.receive(TIMEOUT) {
                    Some(res) => {
                        let obj = json::parse(&res).unwrap();
                        match obj["@type"].as_str().unwrap() {
                            "updateAuthorizationState" => {
                                let astate = &obj["authorization_state"];
                                println!("astate {}", &astate);
                                match astate["@type"].as_str().unwrap() {
                                    "authorizationStateReady" => {
                                        //TODO: store auth credentials
                                        setup_interface(&tdlib);
                                    }
                                    "authorizationStateWaitTdlibParameters" => {
                                        send_tdlib_parameters(&tdlib, api_id, &api_hash);
                                    }
                                    "authorizationStateWaitEncryptionKey" => {
                                        send_check_encryption_key(&tdlib)
                                    }
                                    "authorizationStateWaitPhoneNumber" => {
                                        send_phone_parameters(&tdlib, &phone_number);
                                    }
                                    _ => {
                                        //TODO: verify more formally
                                        if astate.has_key("code_info")
                                            && astate["code_info"].has_key("type")
                                            && astate["code_info"]["type"]["@type"]
                                                == "authenticationCodeTypeTelegramMessage"
                                        {
                                            println!("Input that code king!");
                                            /* TODO: auth code interface so don't have to hardcode it
                                            let mut input_text = String::new();
                                            io::stdin()
                                                .read_line(&mut input_text)
                                                .expect("failed to read from stdin");
                                            */

                                            let input_text = "42101";

                                            let check_auth_code =
                                                CheckAuthenticationCode::builder()
                                                    .code(input_text)
                                                    .build();

                                            tdlib.send(&check_auth_code.to_json().unwrap());
                                        }
                                    }
                                }
                            } // end updateAuthorizationState

                            "updateNewChat" => {
                                let new_chat = &mut obj["chat"].clone();

                                // BEGIN WEIRD STOPGAP I SHOULD PROBABLY RESOLVE
                                new_chat["order"] = json::JsonValue::from("1");
                                new_chat["is_pinned"] = json::JsonValue::from(false);
                                new_chat["is_sponsored"] = json::JsonValue::from(false);
                                new_chat["pinned_message_id"] = json::JsonValue::from(0);
                                //END WEIRD STOPGAP

                                rec_chat_vec
                                    .lock()
                                    .unwrap()
                                    .push(Chat::from_json(new_chat.to_string()).unwrap());
                            }
                            _ => {
                                //println!("Res: {}, {}", res, obj["@type"]);
                            }
                        }
                    }
                    None => {
                        println!("There was an error!");
                    }
                }
            }
        });

        let events = Events::new();
        let ui_chat_vec = chat_list.chat_vec.clone();
        {
            let stdout = io::stdout().into_raw_mode()?;
            let stdout = MouseTerminal::from(stdout);
            let stdout = AlternateScreen::from(stdout);
            let backend = TermionBackend::new(stdout);
            let mut terminal = Terminal::new(backend)?;
            let mut selected_block = 0;

            let selected_style: Style = Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD);
            let _unselected_style: Style = Style::default().fg(Color::White);

            terminal.clear()?;
            loop {
                terminal.draw(|f| {
                    let size = f.size();

                    let chunks = Layout::default()
                        .direction(Direction::Vertical)
                        .margin(1)
                        .constraints(
                            [Constraint::Percentage(85), Constraint::Percentage(15)].as_ref(),
                        )
                        .split(size);
                    let chat_chunks = Layout::default()
                        .direction(Direction::Horizontal)
                        .margin(1)
                        .constraints(
                            [Constraint::Percentage(20), Constraint::Percentage(80)].as_ref(),
                        )
                        .split(chunks[0]);
                    let mut chat_titles = vec::Vec::new();
                    {
                        let local_chat_vec = ui_chat_vec.lock().unwrap().clone();
                        for i in 0..chat_list.get_len().unwrap() {
                            let chat = local_chat_vec.get(i as usize).unwrap();
                            let mut chat_list_item =
                                ListItem::new(Text::from(String::from(chat.title())));
                            if chat_list.selected_index == i {
                                chat_list_item = chat_list_item.style(selected_style);
                            }

                            chat_titles.push(chat_list_item);
                        }
                    }

                    let mut chats_block = List::new(chat_titles)
                        .block(Block::default().title(chat_list.name).borders(Borders::ALL));
                    let mut chat_block =
                        Block::default().title("Current Chat").borders(Borders::ALL);

                    let mut input_block = Block::default().title("Input").borders(Borders::ALL);
                    let input = Paragraph::new(input_str.as_ref())
                        .block(Block::default().borders(Borders::ALL).title("Input"));

                    match selected_block {
                        0 => chats_block = chats_block.style(selected_style),
                        1 => chat_block = chat_block.style(selected_style),
                        2 => input_block = input_block.style(selected_style),
                        _ => (),
                    }

                    f.render_widget(input_block, chunks[1]);
                    f.render_widget(chats_block, chat_chunks[0]);
                    f.render_widget(chat_block, chat_chunks[1]);
                    f.render_widget(input, chunks[1]);
                })?;
                let enext = match events.next() {
                    Ok(eve) => eve,
                    Err(_e) => return Err(Error::new(ErrorKind::Other, "oh no!")),
                };
                if let Event::Input(input) = enext {
                    match input {
                        Key::Char('\n') => {
                            //app.messages.push(app.input.drain(..).collect());
                            return Ok(());
                        }
                        Key::Char('\t') => {
                            //app.messages.push(app.input.drain(..).collect());
                            selected_block = (selected_block + 1) % NUM_BLOCKS;
                        }
                        Key::Ctrl('\t') => {
                            selected_block = (selected_block - 1) % NUM_BLOCKS;
                        }
                        Key::Char(c) => {
                            input_str.push(c);
                        }
                        Key::Backspace => {
                            input_str.pop();
                        }
                        _ => {}
                    }
                }
            }
        }
        //let _rec_result = rec_thread.join().unwrap();
    })
    .unwrap()
}

fn read_info() -> io::Result<(i64, String, String)> {
    let file = File::open("info.txt")?;
    let mut reader = BufReader::new(file);

    let mut api_id = String::new();
    let mut api_hash = String::new();
    let mut phone_number = String::new();

    reader.read_line(&mut api_id)?;
    reader.read_line(&mut api_hash)?;
    reader.read_line(&mut phone_number)?;

    let api_id: i64 = api_id.trim().parse().unwrap();

    Ok((
        api_id,
        api_hash.trim().to_string(),
        phone_number.trim().to_string(),
    ))
}

fn setup_interface(tdlib: &Tdlib) {
    let chat_list = ChatList::default();
    let chat_list_req = GetChats::builder()
        .chat_list(chat_list)
        .offset_order(0)
        .offset_chat_id(0)
        .limit(255)
        .build();
    tdlib.send(&chat_list_req.to_json().unwrap());
}

fn send_tdlib_parameters(tdlib: &Tdlib, api_id: i64, api_hash: &str) {
    let set_tdlib_parameters = SetTdlibParameters::builder()
        .parameters(
            TdlibParameters::builder()
                .use_test_dc(false)
                .database_directory("/tmp/td")
                .files_directory("/tmp/td")
                .use_file_database(false)
                .api_id(api_id)
                .api_hash(api_hash)
                .system_language_code("en")
                .device_model("computer")
                .application_version("0.0.1")
                .build(),
        )
        .build();

    tdlib.send(&set_tdlib_parameters.to_json().unwrap());
}

fn send_check_encryption_key(tdlib: &Tdlib) {
    let check_enc_key = CheckDatabaseEncryptionKey::builder().build();
    tdlib.send(&check_enc_key.to_json().unwrap());
}

fn send_phone_parameters(tdlib: &Tdlib, phone_number: &str) {
    let phone_parameters = SetAuthenticationPhoneNumber::builder()
        .phone_number(phone_number)
        .settings(PhoneNumberAuthenticationSettings::builder().build())
        .build();

    tdlib.send(&phone_parameters.to_json().unwrap());
}
