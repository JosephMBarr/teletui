mod event;
use crossbeam::thread;
use event::{Event, Events};
use rtdlib::types::*;
use rtdlib::Tdlib;
use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io::{self, BufRead, BufReader, Error, ErrorKind};
use std::sync::{Arc, Mutex};
use std::vec;
use termion::{event::Key, input::MouseTerminal, raw::IntoRawMode, screen::AlternateScreen};
use tui::{
    backend::TermionBackend,
    layout::{Constraint, Corner, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Span, Spans, Text},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Terminal,
};

const TIMEOUT: f64 = 0.5;
const DEBUG_LEVEL: i64 = 0;
const DO_DEBUG: bool = false;
const NUM_CHATS: i64 = 20;
const COLORS: [Color; 13] = [
    Color::Red,
    Color::Green,
    Color::Yellow,
    Color::Blue,
    Color::Magenta,
    Color::Cyan,
    Color::Gray,
    Color::LightRed,
    Color::LightGreen,
    Color::LightYellow,
    Color::LightBlue,
    Color::LightMagenta,
    Color::LightCyan,
];

enum InputMode {
    Normal,
    Insert,
}
enum TBlocks {
    ChatList,
    CurrChat,
    Input,
}
struct App {
    curr_mode: InputMode,
    outgoing_queue: Arc<Mutex<VecDeque<String>>>,
    users: Arc<Mutex<HashMap<i64, TUser>>>,
}
struct Chats {
    chat_vec: Arc<Mutex<Vec<TChat>>>,
    name: &'static str,
    selected_index: i32,
}
#[derive(Clone)]
struct TUser {
    u: User,
    color: Color,
}

#[derive(Clone)]
struct TChat {
    history: Arc<Mutex<Vec<Message>>>,
    chat: Chat,
}

impl TChat {
    fn retrieve_history(&self, queue: &Arc<Mutex<VecDeque<String>>>, start_id: i64, limit: i64) {
        let chat_history_req = GetChatHistory::builder()
            .chat_id(self.chat.id())
            .from_message_id(start_id)
            .limit(limit)
            .only_local(false)
            .build();

        queue
            .lock()
            .unwrap()
            .push_back(chat_history_req.to_json().unwrap());
    }
}

trait TBlock {
    fn new(name: &'static str) -> Self;

    fn scroll_down(&mut self, queue: &Arc<Mutex<VecDeque<String>>>);
    fn scroll_up(&mut self, queue: &Arc<Mutex<VecDeque<String>>>);
    fn get_len(&self) -> Result<i32, io::Error> {
        Ok(-1)
    }
    fn handle_input_insert(
        &mut self,
        queue: &Arc<Mutex<VecDeque<String>>>,
        input: &termion::event::Key,
    ) {
    }
    fn handle_input_normal(
        &mut self,
        queue: &Arc<Mutex<VecDeque<String>>>,
        input: &termion::event::Key,
    ) {
    }
}
impl Chats {
    fn select_chat(&self, index: i32, queue: &Arc<Mutex<VecDeque<String>>>) {
        let curr_chat = self
            .chat_vec
            .lock()
            .unwrap()
            .get(index as usize)
            .unwrap()
            .clone();
        if (curr_chat.history.lock().unwrap().len() as i64) < NUM_CHATS {
            curr_chat.retrieve_history(queue, 0, NUM_CHATS);
        }
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
    fn scroll_up(&mut self, queue: &Arc<Mutex<VecDeque<String>>>) {
        self.selected_index = (self.selected_index - 1) % self.get_len().unwrap();
        if self.selected_index < 0 {
            self.selected_index = self.get_len().unwrap() - 1;
        }
        self.select_chat(self.selected_index, queue)
    }
    fn scroll_down(&mut self, queue: &Arc<Mutex<VecDeque<String>>>) {
        self.selected_index = (self.selected_index + 1) % self.get_len().unwrap();
        self.select_chat(self.selected_index, queue)
    }
    fn handle_input_normal(
        &mut self,
        queue: &Arc<Mutex<VecDeque<String>>>,
        input: &termion::event::Key,
    ) {
        match input {
            Key::Char('j') => self.scroll_down(queue),
            Key::Char('k') => self.scroll_up(queue),
            _ => {}
        }
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
        let mut chat_list = Chats::new("Chats");
        let mut app = App {
            curr_mode: InputMode::Normal,
            outgoing_queue: Arc::new(Mutex::new(VecDeque::new())),
            users: Arc::new(Mutex::new(HashMap::new())),
        };
        let rec_chat_vec = chat_list.chat_vec.clone();
        let rec_outgoing_queue = app.outgoing_queue.clone();
        let rec_users = app.users.clone();
        let _rec_thread = s.spawn(move |_| {
            loop {
                match tdlib.receive(TIMEOUT) {
                    Some(res) => {
                        let mut obj = json::parse(&res).unwrap();
                        match obj["@type"].as_str().unwrap() {
                            "updateAuthorizationState" => {
                                let astate = &obj["authorization_state"];
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

                                            let input_text = "69719";

                                            let check_auth_code =
                                                CheckAuthenticationCode::builder()
                                                    .code(input_text)
                                                    .build();

                                            tdlib.send(&check_auth_code.to_json().unwrap());
                                        }
                                    }
                                }
                            } // end updateAuthorizationState
                            "updateUser" => {
                                let num_users = rec_users.lock().unwrap().len();
                                let new_user = TUser {
                                    color: COLORS[num_users % COLORS.len()],
                                    u: User::from_json(obj["user"].to_string()).unwrap(),
                                };
                                rec_users.lock().unwrap().insert(new_user.u.id(), new_user);
                            }

                            "updateNewChat" => {
                                let new_chat = &mut obj["chat"].clone();

                                // BEGIN WEIRD STOPGAP I SHOULD PROBABLY RESOLVE
                                new_chat["order"] = json::JsonValue::from("1");
                                new_chat["is_pinned"] = json::JsonValue::from(false);
                                new_chat["is_sponsored"] = json::JsonValue::from(false);
                                new_chat["pinned_message_id"] = json::JsonValue::from(0);
                                //END WEIRD STOPGAP

                                let tchat = TChat {
                                    chat: Chat::from_json(new_chat.to_string()).unwrap(),
                                    history: Arc::new(Mutex::new(Vec::new())),
                                };
                                rec_chat_vec.lock().unwrap().push(tchat);
                            }
                            "messages" => {
                                let loc_rec_chat_vec = rec_chat_vec.lock().unwrap().clone();
                                let msg_list = &mut obj["messages"].clone();
                                let msg_count = &mut obj["total_count"].as_usize().unwrap();
                                if *msg_count > 0 {
                                    let chat_id = &msg_list[0]["chat_id"].as_i64().unwrap();
                                    let cur_chat = get_chat_by_id(&loc_rec_chat_vec, *chat_id);
                                    let mut cur_chat_history = cur_chat.history.lock().unwrap();
                                    for cur_msg in msg_list.members_mut() {
                                        // ANOTHER WEIRD STOPGAP
                                        cur_msg["sender_user_id"] =
                                            cur_msg["sender"]["user_id"].clone();
                                        cur_msg["views"] = json::JsonValue::from(1);
                                        let cur_msg = match Message::from_json(cur_msg.to_string())
                                        {
                                            Err(e) => {
                                                eprintln!("woops: {}\n{}", e, cur_msg.to_string());
                                                Message::builder().chat_id(*chat_id).build()
                                            }
                                            Ok(ok) => ok,
                                        };
                                        cur_chat_history.insert(0, cur_msg);
                                    }
                                    //Get more messages if less than minimum have been retrieved and there are some left
                                    if (cur_chat_history.len() as i64) < NUM_CHATS && *msg_count > 0
                                    {
                                        let start_id: i64 = cur_chat_history[0]
                                            .id()
                                            .to_string()
                                            .parse::<i64>()
                                            .unwrap();
                                        cur_chat.retrieve_history(
                                            &rec_outgoing_queue,
                                            start_id,
                                            NUM_CHATS,
                                        );
                                    }
                                }
                            }
                            _ => {
                                if DO_DEBUG {
                                    eprintln!("Received: {}", obj);
                                }
                            }
                        }
                    }
                    None => {
                        //Didn't receive, free to send
                        let sz = rec_outgoing_queue.lock().unwrap().len();
                        for _ in 0..sz {
                            let s = rec_outgoing_queue.lock().unwrap().pop_front().unwrap();
                            eprintln!("request: {}", s);
                            tdlib.send(&s);
                        }
                    }
                }
            }
        });

        let events = Events::new();
        let ui_chat_vec = chat_list.chat_vec.clone();
        let ui_outgoing_queue = app.outgoing_queue.clone();
        let ui_users = app.users.clone();
        {
            let stdout = io::stdout().into_raw_mode()?;
            let stdout = MouseTerminal::from(stdout);
            let stdout = AlternateScreen::from(stdout);
            let backend = TermionBackend::new(stdout);
            let mut terminal = Terminal::new(backend)?;

            let selected_style: Style = Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD);
            let unselected_style: Style = Style::default().fg(Color::White);

            let mut selected_block = TBlocks::ChatList;

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
                    let mut chat_history = vec::Vec::new();
                    {
                        let local_chat_vec = ui_chat_vec.lock().unwrap().clone();
                        let loc_ui_users = ui_users.lock().unwrap();
                        for i in 0..chat_list.get_len().unwrap() {
                            let chat = local_chat_vec.get(i as usize).unwrap();
                            let mut chat_list_item =
                                ListItem::new(Text::from(String::from(chat.chat.title())));
                            if chat_list.selected_index == i {
                                chat_list_item = chat_list_item.style(selected_style);
                                for msg in chat.history.lock().unwrap().clone().into_iter() {
                                    let msg_text = match msg.content().as_message_text() {
                                        Some(s) => s.text().text().to_string(),
                                        None => "[none]".to_string(),
                                    };
                                    let (sender_name, sender_color) =
                                        match loc_ui_users.get(&msg.sender_user_id()) {
                                            Some(u) => (&u.u.first_name()[..], u.color),
                                            None => ("Unknown User", COLORS[0]),
                                        };
                                    let sender_name = sender_name.to_owned() + ": ";
                                    let send_len = sender_name.len();
                                    let chat_box_width: usize =
                                        (chat_chunks[1].right() - chat_chunks[1].left() - 2).into();
                                    let mut newline_index = chat_box_width;
                                    let mut first_line = vec![Span::styled(
                                        sender_name,
                                        Style::default().fg(sender_color),
                                    )];
                                    let text_style = Style::default().fg(Color::White);

                                    let mut rest_of_message = Text::from("");
                                    let mut last_index = 0;
                                    while newline_index < send_len + msg_text.len() - 1 {
                                        //Break on word
                                        while msg_text
                                            .chars()
                                            .nth(newline_index - send_len)
                                            .unwrap()
                                            != ' '
                                            && newline_index > 0
                                        {
                                            newline_index -= 1;
                                        }
                                        let replace_index = newline_index - send_len;
                                        let msg_slice =
                                            msg_text[last_index..replace_index].to_owned();
                                        if last_index == 0 {
                                            first_line.push(Span::styled(msg_slice, text_style));
                                        } else {
                                            rest_of_message
                                                .extend(Text::styled(msg_slice, text_style));
                                        }
                                        last_index = replace_index + 1;
                                        newline_index = last_index + chat_box_width;
                                    }
                                    let msg_slice = msg_text[last_index..].to_owned();
                                    if last_index == 0 {
                                        first_line.push(Span::styled(msg_slice, text_style));
                                    } else {
                                        rest_of_message.extend(Text::from(msg_slice));
                                    }

                                    let mut formatted_msg = Text::from(Spans::from(first_line));
                                    formatted_msg.extend(rest_of_message);
                                    chat_history.push(ListItem::new(formatted_msg));
                                }
                            } else {
                                chat_list_item = chat_list_item.style(unselected_style);
                            }

                            chat_titles.push(chat_list_item);
                        }
                    }

                    let mut chats_block = List::new(chat_titles)
                        .block(Block::default().title(chat_list.name).borders(Borders::ALL));
                    let mut chat_block = List::new(chat_history)
                        .block(Block::default().title("Current Chat").borders(Borders::ALL))
                        .start_corner(Corner::BottomRight);

                    let mut input_block = Block::default().title("Input").borders(Borders::ALL);
                    let input = Paragraph::new(input_str.as_ref())
                        .block(Block::default().borders(Borders::ALL).title("Input"));

                    match selected_block {
                        TBlocks::ChatList => chats_block = chats_block.style(selected_style),
                        TBlocks::CurrChat => chat_block = chat_block.style(selected_style),
                        TBlocks::Input => input_block = input_block.style(selected_style),
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
                    match app.curr_mode {
                        InputMode::Normal => match input {
                            Key::F(1) => {
                                return Ok(());
                            }
                            Key::Char('\t') => {
                                selected_block = match selected_block {
                                    TBlocks::ChatList => TBlocks::CurrChat,
                                    TBlocks::CurrChat => TBlocks::Input,
                                    TBlocks::Input => TBlocks::ChatList,
                                }
                            }
                            Key::Esc => {}
                            /*
                            Key::Char(c) => {
                                input_str.push(c);
                            }
                            Key::Backspace => {
                                input_str.pop();
                            }
                            */
                            _ => {
                                if DO_DEBUG {
                                    terminal.clear();
                                }
                                match selected_block {
                                    TBlocks::ChatList => {
                                        chat_list.handle_input_normal(&ui_outgoing_queue, &input)
                                    }
                                    _ => {}
                                }
                            }
                        },
                        InputMode::Insert => match input {
                            Key::Esc => {
                                app.curr_mode = InputMode::Normal;
                            }
                            _ => match selected_block {
                                TBlocks::ChatList => {
                                    chat_list.handle_input_insert(&ui_outgoing_queue, &input)
                                }
                                _ => {}
                            },
                        },
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
fn get_chat_by_id(chat_vec: &Vec<TChat>, chat_id: i64) -> &TChat {
    let mut counter = 0;
    loop {
        let check_chat = chat_vec.get(counter).unwrap();
        if check_chat.chat.id() == chat_id {
            return check_chat;
        }
        if counter >= chat_vec.len() {
            panic!("wrong chats");
        } else {
            counter += 1;
        }
    }
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
