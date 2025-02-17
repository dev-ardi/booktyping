use chrono::{serde::ts_nanoseconds, DateTime, Utc};
use deunicode::deunicode;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{error, fs, fs::File, io::Read, io::Seek, io::Write};

pub const DEFAULT_TEXT_WIDTH_PERCENT: u16 = 60;
pub const FULL_TEXT_WIDTH_PERCENT: u16 = 95;
const STARTING_SAMPLE_SIZE: usize = 100;

/// Application result type.
pub type AppResult<T> = std::result::Result<T, Box<dyn error::Error>>;

/// Application.
#[derive(Debug)]
pub struct App {
    /// Is the application running?
    pub running: bool,
    book_text: String,
    keypress_log: File,
    test_log: File,
    pub book_lines: Vec<String>,
    pub line_index: Vec<(usize, usize)>,
    pub sample_start_index: usize,
    pub sample_len: usize,
    start_time: DateTime<Utc>,
    pub cur_char: usize,
    pub following_typing: bool,
    pub display_line: usize,
    pub text_width_percent: u16,
    pub terminal_width: u16,
    pub full_text_width: bool,
}

impl App {
    /// Constructs a new instance of [`App`].
    pub fn new(book_title: &str, terminal_width: u16) -> AppResult<Self> {
        let book_text = App::load_book(book_title)?;

        let _ = fs::create_dir(
            dirs::home_dir()
                .unwrap()
                .join(".booktyping")
                .join(book_title),
        );

        let mut test_log = App::get_test_log(book_title)?;

        let (sample_start_index, sample_len) = App::get_next_sample(&mut test_log, &book_text)?;

        let mut ret = Self {
            running: true,
            keypress_log: App::get_keypress_log(book_title)?,
            start_time: Utc::now(),
            cur_char: 0,
            test_log,
            book_text,
            sample_start_index,
            sample_len,
            terminal_width,
            following_typing: true,
            text_width_percent: DEFAULT_TEXT_WIDTH_PERCENT,
            full_text_width: false,
            book_lines: Default::default(),
            line_index: Default::default(),
            display_line: Default::default(),
        };

        ret.generate_lines();

        Ok(ret)
    }

    /// Set running to false to quit the application.
    pub fn quit(&mut self) {
        self.running = false;
    }

    pub fn handle_char(&mut self, c: char) -> AppResult<()> {
        if !self.following_typing {
            self.following_typing = true;
        }
        let correct = c
            == self
                .book_text
                .chars()
                .nth(self.sample_start_index + self.cur_char)
                .unwrap();

        if correct {
            self.cur_char += 1
        }
        if !correct || self.cur_char == self.sample_len {
            self.log_test(correct)?;
            self.start_time = Utc::now();
            (self.sample_start_index, self.sample_len) =
                App::get_next_sample(&mut self.test_log, &self.book_text)?;

            self.cur_char = 0;
        }

        let log_entry = serde_json::to_vec(&KeyPress {
            correct,
            key: c,
            time: Utc::now(),
        })
        .unwrap();
        self.keypress_log.write_all(&log_entry)?;
        Ok(())
    }

    fn load_book(book_title: &str) -> AppResult<String> {
        Ok(deunicode(
            &Regex::new(r"\s+")
                .unwrap()
                .replace_all(
                    &fs::read_to_string(
                        dirs::home_dir()
                            .unwrap()
                            .join(".booktyping")
                            .join(format!("{}.txt", book_title)),
                    )?
                    .trim(),
                    " ",
                )
                .to_string(),
        ))
    }

    fn get_keypress_log(book_title: &str) -> AppResult<fs::File> {
        Ok(fs::OpenOptions::new().create(true).append(true).open(
            dirs::home_dir()
                .unwrap()
                .join(".booktyping")
                .join(book_title)
                .join("keypresses.json"),
        )?)
    }

    fn get_test_log(book_title: &str) -> AppResult<fs::File> {
        Ok(fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(
                dirs::home_dir()
                    .unwrap()
                    .join(".booktyping")
                    .join(book_title)
                    .join("tests.json"),
            )?)
    }

    pub fn generate_lines(&mut self) {
        let max_line_len =
            (self.terminal_width as f64 * (self.text_width_percent as f64 / 100.0)) as usize;
        let mut lines = Vec::new();
        let mut line_index: Vec<(usize, usize)> = Vec::new();
        let mut line = "".to_owned();
        let mut word = "".to_owned();
        let mut row_i = 0;
        let mut column_i = 0;

        for c in self.book_text.chars() {
            word.push(c);
            if c == ' ' {
                if line.len() + word.len() < max_line_len {
                    line.push_str(&word);
                } else {
                    lines.push(line);
                    line = word.to_owned();
                    row_i += 1;
                    column_i = 0;
                }
                for _ in 0..word.len() {
                    line_index.push((row_i, column_i));
                    column_i += 1;
                }
                word = "".to_owned();
            }
        }
        if line.len() + word.len() < max_line_len {
            line.push_str(&word);
            lines.push(line);
        } else {
            lines.push(line);
            lines.push(word.clone());
            row_i += 1;
        }
        for _ in 0..word.len() {
            line_index.push((row_i, column_i));
            column_i += 1;
        }

        self.book_lines = lines;
        self.display_line = line_index.get(self.sample_start_index).unwrap().0; //TODO allow for resize while scrolled
        self.line_index = line_index;
    }

    fn get_next_sample(test_log: &mut File, book_text: &str) -> AppResult<(usize, usize)> {
        let mut string = String::new();
        test_log.seek(std::io::SeekFrom::Start(0))?;
        test_log.read_to_string(&mut string)?;
        let tests: Vec<Test> = serde_json::from_str(&string).unwrap_or(Vec::new());

        let mut start_index = 0;
        for t in &tests {
            if t.succeeded && t.end_index > start_index {
                start_index = t.end_index;
            }
        }

        let avg_50 = tests
            .iter()
            .map(|t| t.end_index - t.start_index)
            .filter(|&len| len > 5)
            .rev()
            .take(50)
            .sum::<usize>()
            / 50;
        let max_10 = tests
            .iter()
            .map(|t| t.end_index - t.start_index)
            .filter(|&len| len > 5)
            .rev()
            .take(10)
            .max()
            .unwrap_or(STARTING_SAMPLE_SIZE);
        let best = usize::max(avg_50, max_10) + 5;

        let wrong_num = tests
            .iter()
            .rev()
            .take_while(|t| !t.succeeded)
            .map(|t| t.end_index - t.start_index)
            .filter(|&len| len > 5)
            .count();

        let full = book_text
            .chars()
            .skip(start_index)
            .take(best)
            .collect::<String>();

        let len = full
            .split_whitespace()
            .rev()
            .skip(usize::max(wrong_num, 1))
            .collect::<Vec<_>>()
            .join(" ")
            .len()
            + 1;

        let start_index = usize::min(start_index, book_text.len() - 1);
        let len = usize::min(len, book_text.len() - start_index - 1);
        Ok((start_index, len))
    }

    pub fn get_rolling_average(&mut self) -> AppResult<usize> {
        let mut string = String::new();
        self.test_log.seek(std::io::SeekFrom::Start(0))?;
        self.test_log.read_to_string(&mut string)?;
        let tests: Vec<Test> = serde_json::from_str(&string).unwrap_or(Vec::new());

        Ok(tests
            .iter()
            .map(|t| t.end_index - t.start_index)
            .filter(|&len| len > 5)
            .rev()
            .take(10)
            .sum::<usize>()
            / 10)
    }

    fn log_test(&mut self, succeeded: bool) -> AppResult<()> {
        let mut string = String::new();
        self.test_log.seek(std::io::SeekFrom::Start(0))?;
        self.test_log.read_to_string(&mut string)?;
        let mut tests: Vec<Test> = serde_json::from_str(&string).unwrap_or(Vec::new());
        tests.push(Test {
            succeeded,
            start_index: self.sample_start_index,
            end_index: self.sample_start_index + self.cur_char,
            started: self.start_time,
            completed: Utc::now(),
        });
        self.test_log.seek(std::io::SeekFrom::Start(0))?;
        self.test_log.write_all(&serde_json::to_vec(&tests)?)?;
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
struct KeyPress {
    correct: bool,
    key: char,
    #[serde(with = "ts_nanoseconds")]
    time: DateTime<Utc>,
}

#[derive(Serialize, Deserialize)]
struct Test {
    succeeded: bool,
    start_index: usize,
    end_index: usize,
    #[serde(with = "ts_nanoseconds")]
    started: DateTime<Utc>,
    #[serde(with = "ts_nanoseconds")]
    completed: DateTime<Utc>,
}
