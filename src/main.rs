// TODO: figure out how to scroll down properly
extern crate chrono;
use chrono::prelude::*;
mod event;
use crossbeam::thread;
use event::{Event, Events};
use rtdlib::types::*;
use rtdlib::Tdlib;
use serde_json::{json, Value};
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
const MARGIN: u16 = 1;
const DEBUG_LEVEL: i64 = 0;
const DO_DEBUG: bool = false;
const CODE_ARG: &str = "--code=";
const NO_CODE_PROVIDED: &str = "Please re-run with --code={{code}}";
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

#[derive(Clone)]
enum ScrollDirection {
    Up,
    Down,
}
#[derive(Clone)]
enum InputMode {
    Normal,
    Insert,
}
enum TBlocks {
    ChatList,
    CurrChat,
    Input,
}
#[derive(Clone)]
struct App {
    curr_mode: InputMode,
    outgoing_queue: Arc<Mutex<VecDeque<String>>>,
    users: Arc<Mutex<HashMap<i64, TUser>>>,
    chat_list: Chats,
    input_box: InputBox,
}
#[derive(Clone)]
struct InputBox {
    input_str: String,
    name: &'static str,
}
#[derive(Clone)]
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
    end_of_history: bool,
    retrieving: i64,
    viewport_height: usize,
    bottom_index: usize,
    last_scroll_direction: ScrollDirection,
}
impl App {}

impl TChat {
    fn retrieve_history(
        &mut self,
        queue: &Arc<Mutex<VecDeque<String>>>,
        start_id: i64,
        limit: i64,
    ) {
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

    fn get_oldest_id(&self) -> i64 {
        let hc = self.history.lock().unwrap();
        if hc.len() == 0 {
            return 0;
        }
        hc[hc.len() - 1].id().to_string().parse::<i64>().unwrap()
    }
    fn from_json(j: String) -> TChat {
        TChat {
            chat: Chat::from_json(j).unwrap(),
            history: Arc::new(Mutex::new(Vec::new())),
            end_of_history: false,
            retrieving: -1,
            viewport_height: 0,
            bottom_index: 0,
            last_scroll_direction: ScrollDirection::Up,
        }
    }
}

trait TBlock {
    fn new(name: &'static str) -> Self;

    fn scroll_down(&mut self) {}
    fn scroll_up(&mut self) {}
    fn page_down(&mut self) {}
    fn page_up(&mut self) {}
    fn get_len(&self) -> Result<i32, io::Error> {
        Ok(-1)
    }
    fn handle_input_insert(
        &mut self,
        _queue: &Arc<Mutex<VecDeque<String>>>,
        _input: &termion::event::Key,
        _cur_chat_id: i64,
    ) {
    }
    fn handle_input_normal(
        &mut self,
        _queue: &Arc<Mutex<VecDeque<String>>>,
        input: &termion::event::Key,
    ) {
        match input {
            Key::Char('j') => self.scroll_down(),
            Key::Char('k') => self.scroll_up(),
            Key::Ctrl('f') => self.page_down(),
            Key::Ctrl('b') => self.page_up(),
            _ => {}
        }
    }
}
impl Chats {
    fn get_chat_by_id(&mut self, chat_id: i64) -> Option<TChat> {
        for chat in self.chat_vec.clone().lock().unwrap().iter() {
            if chat.chat.id() == chat_id {
                return Some(chat.clone());
            }
        }
        None
    }
}

impl InputBox {
    fn send_message(&mut self, cur_chat_id: i64, queue: &Arc<Mutex<VecDeque<String>>>) {
        let msg = InputMessageContent::InputMessageText(
            InputMessageText::builder()
                .text(FormattedText::builder().text(self.input_str.clone()))
                .build(),
        );
        let req = SendMessage::builder()
            .chat_id(cur_chat_id)
            .input_message_content(msg)
            .build();
        queue.lock().unwrap().push_back(req.to_json().unwrap());
        //TODO: Error checking
        self.input_str.clear();
    }
}
impl TBlock for InputBox {
    fn new(name: &'static str) -> InputBox {
        InputBox {
            input_str: String::new(),
            name: name,
        }
    }
    fn handle_input_insert(
        &mut self,
        queue: &Arc<Mutex<VecDeque<String>>>,
        input: &termion::event::Key,
        cur_chat_id: i64,
    ) {
        match input {
            Key::Char('\n') => {
                self.send_message(cur_chat_id, queue);
            }
            Key::Char(input) => {
                self.input_str.push(*input);
            }
            Key::Backspace => {
                self.input_str.pop();
            }

            _ => {}
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
    fn scroll_up(&mut self) {
        self.selected_index = (self.selected_index - 1) % self.get_len().unwrap();
        if self.selected_index < 0 {
            self.selected_index = self.get_len().unwrap() - 1;
        }
    }
    fn scroll_down(&mut self) {
        self.selected_index = (self.selected_index + 1) % self.get_len().unwrap();
    }
    fn page_down(&mut self) {}
    fn page_up(&mut self) {}
}
impl TBlock for TChat {
    fn new(_name: &'static str) -> TChat {
        TChat {
            chat: Chat::builder().build(),
            history: Arc::new(Mutex::new(Vec::new())),
            end_of_history: false,
            retrieving: -1,
            viewport_height: 0,
            bottom_index: 0,
            last_scroll_direction: ScrollDirection::Up,
        }
    }
    fn page_up(&mut self) {
        self.last_scroll_direction = ScrollDirection::Up;
        self.bottom_index += self.viewport_height;
    }
    //TODO: fiddle with scrolling off-by-one
    fn page_down(&mut self) {
        self.last_scroll_direction = ScrollDirection::Down;
        if (self.bottom_index as i64 - self.viewport_height as i64) <= self.viewport_height as i64 {
            self.last_scroll_direction = ScrollDirection::Up;
        }
        if (self.bottom_index as i64 - self.viewport_height as i64) <= 0 {
            self.bottom_index = 0;
            return;
        }
        self.bottom_index -= self.viewport_height;
    }
    fn scroll_up(&mut self) {
        self.last_scroll_direction = ScrollDirection::Up;
        self.bottom_index += 1;
    }
    fn scroll_down(&mut self) {
        self.last_scroll_direction = ScrollDirection::Down;
        if (self.bottom_index as i64 - self.viewport_height as i64) <= self.viewport_height as i64 {
            self.last_scroll_direction = ScrollDirection::Up;
        }
        if self.bottom_index as i64 <= 0 {
            self.bottom_index = 0;
            return;
        }
        self.bottom_index -= 1;
    }
}

fn main() {
    let tdlib = Tdlib::new();
    let set_verbosity_level = SetLogVerbosityLevel::builder()
        .new_verbosity_level(DEBUG_LEVEL)
        .build();
    tdlib.send(&set_verbosity_level.to_json().unwrap());

    //Main listening loop
    thread::scope(|s| {
        let mut app = App {
            curr_mode: InputMode::Normal,
            outgoing_queue: Arc::new(Mutex::new(VecDeque::new())),
            users: Arc::new(Mutex::new(HashMap::new())),
            chat_list: Chats::new("Chats"),
            input_box: InputBox::new("Input"),
        };
        let mut rec_app = app.clone();
        let _rec_thread = s.spawn(move |_| {
            td_thread(&tdlib, &mut rec_app);
        });

        ui_thread(&mut app).unwrap();
    })
    .unwrap();
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

fn _send_registration(tdlib: &Tdlib, first_name: &str, last_name: &str) {
    let reg = RegisterUser::builder()
        .first_name(first_name)
        .first_name(last_name)
        .build();

    tdlib.send(&reg.to_json().unwrap());
}

fn td_thread(tdlib: &Tdlib, app: &mut App) {
    let (api_id, api_hash, phone_number) = read_info().unwrap();
    loop {
        match tdlib.receive(TIMEOUT) {
            Some(res) => {
                let mut obj: Value = serde_json::from_str(&res).unwrap();
                if DO_DEBUG {
                    eprintln!("Received: {:?}", obj.get("@type"));
                }
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
                            "authorizationStateWaitCode" => {
                                let input_code = match get_arg(CODE_ARG) {
                                    Some(c) => c,
                                    None => {
                                        eprintln!("{}", NO_CODE_PROVIDED);
                                        std::process::exit(0);
                                    }
                                };
                                let check_auth_code = CheckAuthenticationCode::builder()
                                    .code(input_code.trim())
                                    .build();

                                tdlib.send(&check_auth_code.to_json().unwrap());
                            }
                            _ => {
                                eprintln!("unhandled auth case!: {}", astate);
                            }
                        }
                    } // end updateAuthorizationState
                    "updateUser" => {
                        let num_users = app.users.lock().unwrap().len();
                        let new_user = TUser {
                            color: COLORS[num_users % COLORS.len()],
                            u: User::from_json(obj["user"].to_string()).unwrap(),
                        };
                        app.users.lock().unwrap().insert(new_user.u.id(), new_user);
                    }

                    "updateNewChat" => {
                        let new_chat = &mut obj["chat"];

                        // BEGIN WEIRD STOPGAP I SHOULD PROBABLY RESOLVE
                        new_chat["order"] = serde_json::from_str("1").unwrap();
                        new_chat["is_pinned"] = serde_json::from_value(json!(false)).unwrap();
                        new_chat["is_sponsored"] = serde_json::from_value(json!(false)).unwrap();
                        new_chat["pinned_message_id"] = serde_json::from_value(json!(0)).unwrap();
                        //END WEIRD STOPGAP

                        let tchat = TChat::from_json(new_chat.to_string());
                        app.chat_list.chat_vec.lock().unwrap().push(tchat);
                    }
                    "updateNewMessage" => {
                        eprintln!("new message {}", obj);
                        let msg = &mut obj["message"];
                        let chat_id = match msg["chat_id"].as_i64() {
                            Some(ok) => ok,
                            None => panic!("Couldn't get id: {}", msg),
                        };
                        let cur_chat = &mut app.chat_list.get_chat_by_id(chat_id).unwrap();
                        let mut cur_chat_history = cur_chat.history.lock().unwrap();
                        let cur_msg = parse_msg(msg, chat_id);
                        //place at start
                        cur_chat_history.insert(0, cur_msg);
                    }
                    "messages" => {
                        let msg_count = obj["total_count"].as_u64().unwrap();
                        let msg_list = &mut obj["messages"];
                        let chat_id = match msg_list[0]["chat_id"].as_i64() {
                            Some(ok) => ok,
                            None => panic!("Couldn't get id: {}", msg_list),
                        };
                        let cur_chat = &mut app.chat_list.get_chat_by_id(chat_id).unwrap();
                        if msg_count > 0 {
                            let mut cur_chat_history = cur_chat.history.lock().unwrap();
                            for cur_msg in msg_list.as_array_mut().unwrap() {
                                let cur_msg = parse_msg(cur_msg, chat_id);
                                cur_chat_history.push(cur_msg);
                            }
                        } else {
                            cur_chat.end_of_history = true;
                        }
                    }
                    _ => {}
                }
            }
            None => {
                //Didn't receive, free to send
                let sz = app.outgoing_queue.lock().unwrap().len();
                for _ in 0..sz {
                    let s = app.outgoing_queue.lock().unwrap().pop_front().unwrap();
                    tdlib.send(&s);
                }
            }
        }
    }
}

fn ui_thread(app: &mut App) -> Result<(), std::io::Error> {
    let selected_style: Style = Style::default()
        .fg(Color::Blue)
        .add_modifier(Modifier::BOLD);
    let unselected_style: Style = Style::default().fg(Color::White);
    let stdout = io::stdout().into_raw_mode()?;
    let stdout = MouseTerminal::from(stdout);
    let stdout = AlternateScreen::from(stdout);
    let backend = TermionBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let events = Events::new();
    let mut selected_block = TBlocks::ChatList;

    terminal.clear()?;
    let mut chat_box_height = 0;
    let mut chat_box_width: usize = 0;
    let mut start_corner = Corner::BottomLeft;
    loop {
        terminal.draw(|f| {
            let size = f.size();

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(MARGIN)
                .constraints([Constraint::Percentage(85), Constraint::Percentage(15)].as_ref())
                .split(size);
            let chat_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .margin(MARGIN)
                .constraints([Constraint::Percentage(20), Constraint::Percentage(80)].as_ref())
                .split(chunks[0]);
            let mut chat_titles = vec::Vec::new();
            let ui_users = app.users.lock().unwrap();
            chat_box_height = (chat_chunks[1].bottom() - chat_chunks[1].top() - 2 * MARGIN).into();
            chat_box_width = (chat_chunks[1].right() - chat_chunks[1].left() - 2 * MARGIN).into();
            let mut chat_history = vec::Vec::new();
            let mut chat_title = "Current Chat".to_owned();
            for (i, chat) in (app.chat_list.chat_vec)
                .lock()
                .unwrap()
                .iter_mut()
                .enumerate()
            {
                let chat_list_item = ListItem::new(Text::from(String::from(chat.chat.title())));
                if app.chat_list.selected_index != (i as i32) {
                    // Not selected, style as default and skip ahead to next
                    chat_titles.push(chat_list_item.style(unselected_style));
                    continue;
                }
                let (displayed_msgs, history_height) = build_msg_list(
                    &chat,
                    &ui_users,
                    chat_box_width,
                    chat_box_height,
                    &mut chat_history,
                );
                chat_titles.push(chat_list_item.style(selected_style));
                chat.viewport_height = displayed_msgs;
                //TODO: fix end of history
                let oldest_id = chat.get_oldest_id();
                if history_height < chat_box_height.into()
                    && !chat.end_of_history
                    && chat.retrieving != oldest_id
                {
                    chat.retrieving = oldest_id;
                    chat.retrieve_history(&app.outgoing_queue, oldest_id, chat_box_height as i64);
                }
                start_corner = match chat.last_scroll_direction {
                    ScrollDirection::Up => Corner::BottomLeft,
                    ScrollDirection::Down => Corner::TopLeft,
                };
                let extra_info = if chat.chat.type_().is_private() {
                    // Get user and the time they were last seen
                    let recipient_id = chat.chat.type_().as_private().unwrap().user_id();
                    let recipient = ui_users.get(&recipient_id).unwrap();
                    let status = recipient.u.status();
                    if status.is_online() {
                        "online".to_string()
                    } else if status.is_offline() {
                        let date_time = NaiveDateTime::from_timestamp(
                            status.as_offline().unwrap().was_online(),
                            0,
                        );
                        format!("last seen {}", date_time)
                    } else {
                        "unknown".to_string()
                    }
                } else {
                    "some group".to_string()
                };
                chat_title = format!("{}: {}", *chat.chat.title(), extra_info);
            }

            let mut chats_block = List::new(chat_titles).block(
                Block::default()
                    .title(app.chat_list.name)
                    .borders(Borders::ALL),
            );
            let mut chat_block = List::new(chat_history)
                .block(Block::default().title(chat_title).borders(Borders::ALL))
                .start_corner(start_corner);

            let mut input_block = Block::default()
                .title(app.input_box.name)
                .borders(Borders::ALL);
            let input = Paragraph::new(app.input_box.input_str.as_ref())
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
                    Key::Char('i') => app.curr_mode = InputMode::Insert,
                    _ => match selected_block {
                        TBlocks::ChatList => {
                            app.chat_list
                                .handle_input_normal(&app.outgoing_queue, &input);
                        }
                        TBlocks::CurrChat => {
                            app.chat_list
                                .chat_vec
                                .lock()
                                .unwrap()
                                .get_mut(app.chat_list.selected_index as usize)
                                .unwrap()
                                .handle_input_normal(&app.outgoing_queue, &input);
                        }
                        _ => {}
                    },
                },
                InputMode::Insert => {
                    let cur_chat_id = app
                        .chat_list
                        .chat_vec
                        .lock()
                        .unwrap()
                        .get(app.chat_list.selected_index as usize)
                        .unwrap()
                        .chat
                        .id();
                    match input {
                        Key::Esc => app.curr_mode = InputMode::Normal,
                        _ => match selected_block {
                            TBlocks::ChatList => app.chat_list.handle_input_insert(
                                &app.outgoing_queue,
                                &input,
                                cur_chat_id,
                            ),
                            TBlocks::Input => app.input_box.handle_input_insert(
                                &app.outgoing_queue,
                                &input,
                                cur_chat_id,
                            ),

                            _ => {}
                        },
                    }
                }
            }
        }
    }
}
/*
 * Build the message list to be displayed, based on size parameters of chat box
 */

fn build_msg_list(
    chat: &TChat,
    ui_users: &HashMap<i64, TUser>,
    chat_box_width: usize,
    chat_box_height: usize,
    chat_history: &mut Vec<ListItem>,
) -> (usize, usize) {
    // Track total number of messages displayed, for tracking scroll

    let h = chat.history.lock().unwrap();
    let mut temp_vec = vec![Message::builder().build(); chat.bottom_index];
    let chat_slice = match chat.last_scroll_direction {
        ScrollDirection::Up => h[chat.bottom_index..].iter(),
        ScrollDirection::Down => {
            temp_vec.clone_from_slice(&h[..chat.bottom_index]);
            temp_vec.reverse();
            temp_vec.iter()
        }
    };

    let mut history_height = 0;
    // Iterate through the chat hsitory, starting at the bottommost message that is to be displayed
    for msg in chat_slice {
        //TODO: handle more message types
        let msg_text = match msg.content().as_message_text() {
            Some(s) => s.text().text().to_string(),
            None => "[none]".to_string(),
        };
        let (sender_name, sender_color) = match ui_users.get(&msg.sender_user_id()) {
            Some(u) => (u.u.first_name().to_string(), u.color),
            None => ("Unknown User".to_string(), COLORS[0]),
        };
        //let sender_name = "dumbdumb";
        //let sender_color = COLORS[0];
        let send_len = sender_name.chars().count();

        let text_style = Style::default()
            .remove_modifier(Modifier::BOLD)
            .fg(Color::White);
        let full_msg = format!("{}: {}", sender_name, msg_text);
        let lines = textwrap::fill(&full_msg, chat_box_width);
        let mut lis = Text::from("");
        for (i, l) in lines.lines().collect::<Vec<&str>>().iter().enumerate() {
            history_height += 1;
            if i == 0 {
                let mut first_line = vec![Span::styled(
                    sender_name.clone(),
                    Style::default()
                        .remove_modifier(Modifier::BOLD)
                        .fg(sender_color),
                )];
                let msg_slice: String = (*l).chars().skip(send_len).collect();
                first_line.push(Span::styled(msg_slice, text_style));
                lis = Text::from(Spans::from(first_line));
                continue;
            }
            let t = Text::styled((*l).to_owned(), text_style).to_owned();
            lis.extend(t);
        }
        if history_height > chat_box_height {
            break;
        }
        chat_history.push(ListItem::new(lis));
    }
    return (chat_history.len(), history_height);
}
/*
 *   Get specified command line argument
 */

fn get_arg(arg_name: &str) -> Option<String> {
    for arg in std::env::args() {
        if &arg[..arg_name.len()] == arg_name {
            return Some(arg.split("=").collect::<Vec<&str>>()[1].trim().to_string());
        }
    }
    None
}
fn parse_msg(cur_msg: &mut Value, chat_id: i64) -> Message {
    // ANOTHER WEIRD STOPGAP
    cur_msg["sender_user_id"] = cur_msg["sender"]["user_id"].to_owned();
    cur_msg["views"] = json!(1);
    let cur_msg = match Message::from_json(cur_msg.to_string()) {
        Err(_e) => {
            let msg_builder = &mut Message::builder();
            let formatted_str = match cur_msg["content"]["@type"].as_str().unwrap() {
                "messageSticker" => {
                    format!("[{} Sticker]", cur_msg["content"]["sticker"]["emoji"])
                }
                //TODO: more type safety
                "messageText" => match cur_msg["content"].get("web_page") {
                    Some(_c) => {
                        let wp = &cur_msg["content"]["web_page"];
                        format!(
                            "{}\n{}\n{}\n{}",
                            cur_msg["content"]["text"]["text"].as_str().unwrap(),
                            wp["site_name"].as_str().unwrap(),
                            wp["title"].as_str().unwrap(),
                            wp["description"]["text"].as_str().unwrap()
                        )
                    }

                    None => "[none]".to_owned(),
                },
                t => {
                    eprintln!("truly the type is {}", t);
                    "[none]".to_owned()
                }
            };
            msg_builder
                .content(MessageContent::MessageText(
                    MessageText::builder()
                        .text(FormattedText::builder().text(formatted_str))
                        .build(),
                ))
                .chat_id(chat_id)
                .sender_user_id(
                    cur_msg["sender_user_id"]
                        .to_string()
                        .parse::<i64>()
                        .unwrap(),
                )
                .build()
        }
        Ok(ok) => ok,
    };
    return cur_msg;
}
