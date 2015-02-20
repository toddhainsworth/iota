//FIXME: Check unicode support
// stdlib dependencies
use std::cmp;
use std::collections::HashMap;
use std::old_io::{File, Reader, BufferedReader};

// external dependencies
use gapbuffer::GapBuffer;

// local dependencies
use log::{Log, Change, LogEntry};
use utils::is_alpha_or_;
use iterators::{Lines, Chars};
use textobject::{TextObject, Kind, Offset, Anchor};


#[derive(Copy, PartialEq, Eq, Hash, Debug)]
pub enum Mark {
    Cursor(usize),           //For keeping track of cursors.
    DisplayMark(usize),      //For using in determining some display of characters.
}

#[derive(Copy, PartialEq, Eq, Debug)]
pub enum WordEdgeMatch {
    Alphabet,
    Whitespace,
}

#[derive(Copy, PartialEq, Eq, Debug)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,

    // TODO: extract to TextObject::Word - or similar
    LeftWord(WordEdgeMatch),
    RightWord(WordEdgeMatch),

    // TODO: extract to TextObject::Line - or similar
    LineStart,
    LineEnd,
    FirstLine,
    LastLine,
}

pub struct Buffer {
    /// Current buffers text
    text: GapBuffer<u8>,

    /// Table of marked indices in the text
    /// KEY: mark id => VALUE : (absolute index, line index)
    ///
    /// - absolute index is the offset from the start of the buffer
    /// - line index is the offset from the start of the current line
    marks: HashMap<Mark, (usize, usize)>,

    /// Transaction history (used for undo/redo)
    pub log: Log,

    /// Location on disk where the current buffer should be written
    pub file_path: Option<Path>,
}

impl Buffer {

    /// Constructor for empty buffer.
    pub fn new() -> Buffer {
        Buffer {
            file_path: None,
            text: GapBuffer::new(),
            marks: HashMap::new(),
            log: Log::new(),
        }
    }

    /// Constructor for buffer from reader.
    pub fn new_from_reader<R: Reader>(reader: R) -> Buffer {
        let mut buff = Buffer::new();
        if let Ok(contents) = BufferedReader::new(reader).read_to_string() {
            buff.text.extend(contents.bytes());
        }
        buff
    }

    /// Constructor for buffer from file.
    pub fn new_from_file(path: Path) -> Buffer {
        if let Ok(file) = File::open(&path) {
            let mut buff = Buffer::new_from_reader(file);
            buff.file_path = Some(path);
            buff
        } else { Buffer::new() }
    }

    /// Length of the text stored in this buffer.
    pub fn len(&self) -> usize {
        self.text.len() + 1
    }

    /// The x,y coordinates of a mark within the file. None if not a valid mark.
    pub fn get_mark_coords(&self, mark: Mark) -> Option<(usize, usize)> {
        if let Some(idx) = self.get_mark_idx(mark) {
            if let Some(line) = get_line(idx, &self.text) {
                Some((idx - line, range(0, idx).filter(|i| -> bool { self.text[*i] == b'\n' })
                                               .count()))
            } else { None }
        } else { None }
    }

    /// The absolute index of a mark within the file. None if not a valid mark.
    pub fn get_mark_idx(&self, mark: Mark) -> Option<usize> {
        if let Some(&(idx, _)) = self.marks.get(&mark) {
            if idx < self.len() {
                Some(idx)
            } else { None }
        } else { None }
    }

    /// Creates an iterator on the text by chars.
    pub fn chars(&self) -> Chars {
        Chars {
            buffer: &self.text,
            idx: 0,
            forward: true,
        }
    }

    /// Creates an iterator on the text by chars that begins at the specified index.
    pub fn chars_from_idx(&self, idx: usize) -> Option<Chars> {
        if idx < self.len() {
            Some(Chars {
                buffer: &self.text,
                idx: idx,
                forward: true,
            })
        } else { None }
    }

    /// Creates an iterator on the text by chars that begins at the specified mark.
    pub fn chars_from(&self, mark: Mark) -> Option<Chars> {
        if let Some(&(idx, _)) = self.marks.get(&mark) {
            self.chars_from_idx(idx)
        } else { None }
    }

    /// Creates an iterator on the text by lines.
    pub fn lines(&self) -> Lines {
        Lines {
            buffer: &self.text,
            tail: 0,
            head: self.len()
        }
    }

    /// Creates an iterator on the text by lines that begins at the specified mark.
    pub fn lines_from(&self, mark: Mark) -> Option<Lines> {
        if let Some(&(idx, _)) = self.marks.get(&mark) {
            if idx < self.len() {
                Some(Lines {
                    buffer: &self.text,
                    tail: idx,
                    head: self.len(),
                })
            } else { None }
        } else { None }
    }

    /// Return the buffer index of a TextObject
    pub fn get_object_index(&self, obj: TextObject) -> Option<(usize, usize)> {
        // FIXME: don't assume the mark is Cursor
        let mark = Mark::Cursor(0);

        if let Some(tuple) = self.marks.get(&mark) {
            let (index, line_index) = *tuple;
            match obj.kind {
                Kind::Char => self.get_char_index(obj.offset, index, line_index),
                Kind::Line(a) => self.get_line_index(obj.offset, a, index, line_index),
                Kind::Word(a) => self.get_word_index(obj.offset, a, index, line_index),
            }
        } else {
            None
        }
    }

    fn get_nth_char_index_from_index(&self, start: usize, n: usize) -> Option<usize> {
        if let Some(iter) = self.chars_from_idx(start) {
            if let Some((index, _)) = iter.indices().skip(n-1).next() {
                return Some(cmp::max(0, index))
            }
        }
        None
    }

    fn get_char_index(&self, offset: Offset, index: usize, line_index: usize) -> Option<(usize, usize)> {
        let last = self.len() - 1;
        let text = &self.text;
        let new_positions = match offset {
            // FIXME: don't ignore the from_mark here
            Offset::Forward(offset, from_mark) => {
                let absolute_index = index + offset;
                if absolute_index < last {
                    (absolute_index, absolute_index - get_line(absolute_index, text).unwrap())
                } else {
                    (last, last - get_line(last, text).unwrap())
                }
            }

            // FIXME: don't ignore the from_mark here
            Offset::Backward(offset, from_mark) => {
                if index >= offset {
                    let absolute_index = index - offset;
                    (absolute_index, absolute_index - get_line(absolute_index, text).unwrap())
                } else {
                    (0, 0)
                }
            }

            Offset::Absolute(absolute_char_offset) => {
                (absolute_char_offset, absolute_char_offset - get_line(absolute_char_offset, text).unwrap())
            },
        };

        Some(new_positions)
    }

    fn get_line_index(&self, offset: Offset, anchor: Anchor, index: usize, line_index: usize) -> Option<(usize, usize)> {
        let text = &self.text;
        let last = self.len() - 1;
        let new_positions = match offset {
            // FIXME: don't ignore the from_mark here
            Offset::Forward(offset, from_mark) => {
                let nlines = range(index, text.len()).filter(|i| text[*i] == b'\n')
                                                   .take(offset + 1)
                                                   .collect::<Vec<usize>>();

                match anchor {
                    Anchor::Same => {
                        if offset == nlines.len() {
                            (cmp::min(line_index + nlines[offset-1] + 1, last), line_index)
                        } else {
                            (cmp::min(line_index + nlines[offset-1] + 1, nlines[offset]), line_index)
                        }
                    }

                    Anchor::End => {
                        let end_offset = cmp::min(line_index + nlines[offset] + 1, nlines[offset]);
                        (end_offset, end_offset - get_line(end_offset, text).unwrap())
                    }

                    _ => {
                        print!("Unhandled line anchor: {:?} ", anchor);
                        (0, 0)
                    },
                }

                // if offset > nlines.len() {
                //     (last, last - get_line(last, text).unwrap())
                // } else if offset == nlines.len() {
                //     (cmp::min(line_index + nlines[offset-1] + 1, last), line_index)
                // } else if offset == 0 {
                //     let end_offset = cmp::min(line_index + nlines[offset] + 1, nlines[offset]);
                //     (end_offset, end_offset - get_line(end_offset, text).unwrap())
                // } else {
                //     (cmp::min(line_index + nlines[offset-1] + 1, nlines[offset]), line_index)
                // }
            }

            // FIXME: don't ignore the from_mark here
            Offset::Backward(offset, from_mark) => {
                let nlines = range(0, index).rev().filter(|i| text[*i] == b'\n')
                                                .take(offset + 1)
                                                .collect::<Vec<usize>>();

                match anchor {

                    Anchor::Start => {
                        let start_offset = cmp::min(line_index + nlines[offset] + 1, nlines[offset]);
                        (start_offset + 1, 0)
                    }

                    Anchor::Same => {
                        if offset == 0 {
                            (0, 0) // going to start of the first line
                        } else {
                            (cmp::min(line_index, nlines[0]), line_index)
                        }
                    }

                    _ => {
                        print!("Unhandled line anchor: {:?} ", anchor);
                        (0, 0)
                    },
                }

                // if offset == nlines.len() {
                //     if offset == 0 {
                //         // going to the start of the first line
                //         (0, 0)
                //     } else {
                //         (cmp::min(line_index, nlines[0]), line_index)
                //     }
                // } else if offset > nlines.len() {
                //     (0, 0)
                // } else if offset == 0 {
                //     // going to the start of the line
                //     let start_offset = cmp::min(line_index + nlines[offset] + 1, nlines[offset]);
                //     (start_offset + 1, 0)
                // } else {
                //     (cmp::min(line_index + nlines[offset] + 1, nlines[offset-1]), line_index)
                // }
            }

            Offset::Absolute(line_number) => {
                let nlines = range(0, text.len()).filter(|i| text[*i] == b'\n')
                                                 .take(line_number + 1)
                                                 .collect::<Vec<usize>>();
                match anchor {
                    Anchor::Start => {
                        let end_offset = nlines[line_number - 1];
                        let start = get_line(end_offset, text).unwrap();
                        (start, 0)
                    }

                    Anchor::End => {
                        let end_offset = nlines[line_number - 1];
                        (end_offset, end_offset)
                    }

                    _ => {
                        print!("Unhandled line anchor: {:?} ", anchor);
                        (0, 0)
                    },
                }
            }
        };

        Some(new_positions)
    }

    fn get_word_index(&self, offset: Offset, anchor: Anchor, index: usize, line_index: usize) -> Option<(usize, usize)> {
        let last = self.len() - 1;
        let text = &self.text;
        let new_positions = match offset {
            // FIXME: don't ignore the from_mark here
            Offset::Forward(nth_word, from_mark) => {
                // TODO: use anchor to determine this
                let edger = WordEdgeMatch::Whitespace;

                match anchor {
                    Anchor::Start => {
                        // move to the start of nth_word from the mark
                        if let Some(new_index) = get_words(index, nth_word, edger, text) {
                            (new_index, new_index - get_line(new_index, text).unwrap())
                        } else {
                            (last, last - get_line(last, text).unwrap())
                        }
                    }

                    _ => {
                        print!("Unhandled word anchor: {:?} ", anchor);
                        (last, last - get_line(last, text).unwrap())
                    }
                }
                // if let Some(new_idx) = get_words(index, offset, edger, text) {
                //     if new_idx < last {
                //         panic!("1a");
                //         (new_idx, new_idx - get_line(new_idx, text).unwrap())
                //     } else {
                //         panic!("1b");
                //         (last, last - get_line(last, text).unwrap())
                //     }
                // } else {
                //     panic!("2");
                //     (last, last - get_line(last, text).unwrap())
                // }
            }

            // FIXME: don't ignore the from_mark here
            Offset::Backward(nth_word, from_mark) => {
                // TODO: use anchor to determine this
                let edger = WordEdgeMatch::Whitespace;

                match anchor {
                    Anchor::Start => {
                        // move to the start of the nth_word before the mark
                        if let Some(new_index) = get_words_rev(index, nth_word, edger, text) {
                            (new_index, new_index - get_line(new_index, text).unwrap())
                        } else {
                            (0, 0)
                        }
                    }

                    _ => {
                        print!("Unhandled word anchor: {:?} ", anchor);
                        (0, 0)
                    },
                }
                // if let Some(new_idx) = get_words_rev(index, offset, edger, text) {
                //     if new_idx > 0 {
                //         (new_idx, new_idx - get_line(new_idx, text).unwrap())
                //     } else {
                //         (0, 0)
                //     }
                // } else {
                //     (0, 0)
                // }
            }

            // FIXME
            Offset::Absolute(word_number) => {
                // TODO: use anchor to determine this
                let edger = WordEdgeMatch::Whitespace;

                match anchor {
                    Anchor::Start => {
                        let mut new_index = 0;
                        let new_index = get_words(0, word_number - 1, edger, text).unwrap();

                        (new_index, new_index - get_line(new_index, text).unwrap())
                    }

                    _ => {
                        print!("Unhandled word anchor: {:?} ", anchor);
                        (0, 0)
                    },
                }
            }
        };
        Some(new_positions)
    }

    /// Returns the status text for this buffer.
    pub fn status_text(&self) -> String {
        match self.file_path {
            Some(ref path)  =>  format!("{} ", path.display()),
            None            =>  format!("untitled "),
        }
    }

    /// Sets the mark to the location of a given TextObject, if it exists.
    /// Adds a new mark or overwrites an existing mark.
    pub fn set_mark_to_object(&mut self, mark: Mark, obj: TextObject) {
        let last = self.len() - 1;
        let text = &self.text;
        // if let Some(tuple) = self.marks.get_mut(&mark) {
        //     let (index, line_index) = *tuple;
        //     *tuple = match obj.kind {
        //         Kind::Char => {
        //             match offset {
        //                 // FIXME: don't ignore the from_mark here
        //                 Offset::Forward(offset, from_mark) => {
        //                     let absolute_index = index + offset;
        //                     if absolute_index < last {
        //                         (absolute_index, absolute_index - get_line(absolute_index, text).unwrap())
        //                     } else {
        //                         (last, last - get_line(last, text).unwrap())
        //                     }
        //                 }

        //                 // FIXME: don't ignore the from_mark here
        //                 Offset::Backward(offset, from_mark) => {
        //                     if index >= offset {
        //                         let absolute_index = index - offset;
        //                         (absolute_index, absolute_index - get_line(absolute_index, text).unwrap())
        //                     } else {
        //                         (0, 0)
        //                     }
        //                 }

        //                 Offset::Absolute(absolute_char_offset) => {
        //                     (absolute_char_offset, absolute_char_offset - get_line(absolute_char_offset, text).unwrap())
        //                 },
        //             }
        //         }


        //         Kind::Line(anchor) => {
        //             match obj.offset {
        //                 // FIXME: don't ignore the from_mark here
        //                 Offset::Forward(offset, from_mark) => {
        //                     let nlines = range(index, text.len()).filter(|i| text[*i] == b'\n')
        //                                                        .take(offset + 1)
        //                                                        .collect::<Vec<usize>>();

        //                     match anchor {
        //                         Anchor::Same => {
        //                             if offset == nlines.len() {
        //                                 (cmp::min(line_index + nlines[offset-1] + 1, last), line_index)
        //                             } else {
        //                                 (cmp::min(line_index + nlines[offset-1] + 1, nlines[offset]), line_index)
        //                             }
        //                         }

        //                         Anchor::End => {
        //                             let end_offset = cmp::min(line_index + nlines[offset] + 1, nlines[offset]);
        //                             (end_offset, end_offset - get_line(end_offset, text).unwrap())
        //                         }

        //                         _ => {
        //                             print!("Unhandled anchor from: {:?} ", obj);
        //                             (0, 0)
        //                         },
        //                     }

        //                     // if offset > nlines.len() {
        //                     //     (last, last - get_line(last, text).unwrap())
        //                     // } else if offset == nlines.len() {
        //                     //     (cmp::min(line_index + nlines[offset-1] + 1, last), line_index)
        //                     // } else if offset == 0 {
        //                     //     let end_offset = cmp::min(line_index + nlines[offset] + 1, nlines[offset]);
        //                     //     (end_offset, end_offset - get_line(end_offset, text).unwrap())
        //                     // } else {
        //                     //     (cmp::min(line_index + nlines[offset-1] + 1, nlines[offset]), line_index)
        //                     // }
        //                 }

        //                 // FIXME: don't ignore the from_mark here
        //                 Offset::Backward(offset, from_mark) => {
        //                     let nlines = range(0, index).rev().filter(|i| text[*i] == b'\n')
        //                                                     .take(offset + 1)
        //                                                     .collect::<Vec<usize>>();

        //                     match anchor {

        //                         Anchor::Start => {
        //                             let start_offset = cmp::min(line_index + nlines[offset] + 1, nlines[offset]);
        //                             (start_offset + 1, 0)
        //                         }

        //                         Anchor::Same => {
        //                             if offset == 0 {
        //                                 (0, 0) // going to start of the first line
        //                             } else {
        //                                 (cmp::min(line_index, nlines[0]), line_index)
        //                             }
        //                         }

        //                         _ => {
        //                             print!("Unhandled anchor from: {:?} ", obj);
        //                             (0, 0)
        //                         },
        //                     }

        //                     // if offset == nlines.len() {
        //                     //     if offset == 0 {
        //                     //         // going to the start of the first line
        //                     //         (0, 0)
        //                     //     } else {
        //                     //         (cmp::min(line_index, nlines[0]), line_index)
        //                     //     }
        //                     // } else if offset > nlines.len() {
        //                     //     (0, 0)
        //                     // } else if offset == 0 {
        //                     //     // going to the start of the line
        //                     //     let start_offset = cmp::min(line_index + nlines[offset] + 1, nlines[offset]);
        //                     //     (start_offset + 1, 0)
        //                     // } else {
        //                     //     (cmp::min(line_index + nlines[offset] + 1, nlines[offset-1]), line_index)
        //                     // }
        //                 }

        //                 Offset::Absolute(line_number) => {
        //                     let nlines = range(0, text.len()).filter(|i| text[*i] == b'\n')
        //                                                      .take(line_number + 1)
        //                                                      .collect::<Vec<usize>>();
        //                     match anchor {
        //                         Anchor::Start => {
        //                             let end_offset = nlines[line_number - 1];
        //                             let start = get_line(end_offset, text).unwrap();
        //                             (start, 0)
        //                         }

        //                         Anchor::End => {
        //                             let end_offset = nlines[line_number - 1];
        //                             (end_offset, end_offset)
        //                         }

        //                         _ => {
        //                             print!("Unhandled anchor from: {:?} ", obj);
        //                             (0, 0)
        //                         },
        //                     }
        //                 }
        //             }
        //         }


        //         Kind::Word(anchor) => {
        //             match obj.offset {
        //                 // FIXME: don't ignore the from_mark here
        //                 Offset::Forward(nth_word, from_mark) => {
        //                     // TODO: use anchor to determine this
        //                     let edger = WordEdgeMatch::Whitespace;

        //                     match anchor {
        //                         Anchor::Start => {
        //                             // move to the start of nth_word from the mark
        //                             if let Some(new_index) = get_words(index, nth_word, edger, text) {
        //                                 (new_index, new_index - get_line(new_index, text).unwrap())
        //                             } else {
        //                                 (last, last - get_line(last, text).unwrap())
        //                             }
        //                         }

        //                         _ => {
        //                             print!("Unhandled anchor from: {:?} ", obj);
        //                             (last, last - get_line(last, text).unwrap())
        //                         }
        //                     }
        //                     // if let Some(new_idx) = get_words(index, offset, edger, text) {
        //                     //     if new_idx < last {
        //                     //         panic!("1a");
        //                     //         (new_idx, new_idx - get_line(new_idx, text).unwrap())
        //                     //     } else {
        //                     //         panic!("1b");
        //                     //         (last, last - get_line(last, text).unwrap())
        //                     //     }
        //                     // } else {
        //                     //     panic!("2");
        //                     //     (last, last - get_line(last, text).unwrap())
        //                     // }
        //                 }

        //                 // FIXME: don't ignore the from_mark here
        //                 Offset::Backward(nth_word, from_mark) => {
        //                     // TODO: use anchor to determine this
        //                     let edger = WordEdgeMatch::Whitespace;

        //                     match anchor {
        //                         Anchor::Start => {
        //                             // move to the start of the nth_word before the mark
        //                             if let Some(new_index) = get_words_rev(index, nth_word, edger, text) {
        //                                 (new_index, new_index - get_line(new_index, text).unwrap())
        //                             } else {
        //                                 (0, 0)
        //                             }
        //                         }

        //                         _ => {
        //                             print!("Unhandled anchor from: {:?} ", obj);
        //                             (0, 0)
        //                         },
        //                     }
        //                     // if let Some(new_idx) = get_words_rev(index, offset, edger, text) {
        //                     //     if new_idx > 0 {
        //                     //         (new_idx, new_idx - get_line(new_idx, text).unwrap())
        //                     //     } else {
        //                     //         (0, 0)
        //                     //     }
        //                     // } else {
        //                     //     (0, 0)
        //                     // }
        //                 }

        //                 // FIXME
        //                 Offset::Absolute(word_number) => {
        //                     // TODO: use anchor to determine this
        //                     let edger = WordEdgeMatch::Whitespace;

        //                     match anchor {
        //                         Anchor::Start => {
        //                             let mut new_index = 0;
        //                             let new_index = get_words(0, word_number - 1, edger, text).unwrap();

        //                             (new_index, new_index - get_line(new_index, text).unwrap())
        //                         }

        //                         _ => {
        //                             print!("Unhandled anchor from: {:?} ", obj);
        //                             (0, 0)
        //                         },
        //                     }
        //                 }
        //             }
        //         }


        //     }
        // }

        if let Some(tple) = self.get_object_index(obj) {
            self.marks.insert(mark, tple);
        }
    }

    /// Sets the mark to a given absolute index. Adds a new mark or overwrites an existing mark.
    pub fn set_mark(&mut self, mark: Mark, idx: usize) {
        if let Some(line) = get_line(idx, &self.text) {
            if let Some(tuple) = self.marks.get_mut(&mark) {
                *tuple = (idx, idx - line);
                return;
            }
            self.marks.insert(mark, (idx, idx - line));
        }
    }

    /// Shift a mark relative to its position according to the direction given.
    pub fn shift_mark(&mut self, mark: Mark, direction: Direction, amount: usize) {
        let last = self.len() - 1;
        let text = &self.text;
        if let Some(tuple) = self.marks.get_mut(&mark) {
            let (idx, line_idx) = *tuple;
            if let (Some(line), Some(line_end)) = (get_line(idx, text), get_line_end(idx, text)) {
                *tuple = match direction {
                    //For every relative motion of a mark, should return this tuple:
                    //  value 0:    the absolute index of the mark in the file
                    //  value 1:    the index of the mark in its line (unchanged by direct verticle
                    //              traversals)
                    Direction::Left =>  {
                        let n = amount;
                        if idx >= n { (idx - n, idx - n - get_line(idx - n, text).unwrap()) }
                        else { (0, 0) }
                    }
                    Direction::Right =>  {
                        let n = amount;
                        if idx + n < last { (idx + n, idx + n - get_line(idx + n, text).unwrap()) }
                        else { (last, last - get_line(last, text).unwrap()) }
                    }
                    Direction::Up =>  {
                        let n = amount;
                        let nlines = range(0, idx).rev().filter(|i| text[*i] == b'\n')
                                                        .take(n + 1)
                                                        .collect::<Vec<usize>>();
                        if n == nlines.len() { (cmp::min(line_idx, nlines[0]), line_idx) }
                        else if n > nlines.len() { (0, 0) }
                        else { (cmp::min(line_idx + nlines[n] + 1, nlines[n-1]), line_idx) }
                    }
                    Direction::Down =>  {
                        let n = amount;
                        let nlines = range(idx, text.len()).filter(|i| text[*i] == b'\n')
                                                           .take(n + 1)
                                                           .collect::<Vec<usize>>();
                        if n > nlines.len() { (last, last - get_line(last, text).unwrap())
                        } else if n == nlines.len() {
                            (cmp::min(line_idx + nlines[n-1] + 1, last), line_idx)
                        } else { (cmp::min(line_idx + nlines[n-1] + 1, nlines[n]), line_idx) }
                    }
                    Direction::RightWord(edger) =>  {
                        let n_words = amount;
                        if let Some(new_idx) = get_words(idx, n_words, edger, text) {
                            if new_idx < last { (new_idx, new_idx - get_line(new_idx, text).unwrap()) }
                            else { (last, last - get_line(last, text).unwrap()) }
                        } else { (last, last - get_line(last, text).unwrap()) }
                    }
                    Direction::LeftWord(edger)     =>  {
                        let n_words = amount;
                        if let Some(new_idx) = get_words_rev(idx, n_words, edger, text) {
                            if new_idx > 0 { (new_idx, new_idx - get_line(new_idx, text).unwrap()) }
                            else { (0, 0) }
                        } else { (0, 0) }
                    }
                    Direction::LineStart    =>  { (line, 0) }
                    Direction::LineEnd      =>  { (line_end, line_end - line) }
                    Direction::FirstLine    =>  { (0, 0) }
                    Direction::LastLine     =>  { (last, 0) }
                }
            }
        }
    }

    // Remove the chars in the range from start to end
    pub fn remove_range(&mut self, start: usize, end: usize) -> Option<Vec<u8>> {
        let text = &mut self.text;
        let mut transaction = self.log.start(start);
        let mut vec = range(start, end)
            .rev()
            .filter_map(|idx| text.remove(idx).map(|ch| (idx, ch)))
            .inspect(|&(idx, ch)| transaction.log(Change::Remove(idx, ch), idx))
            .map(|(_, ch)| ch)
            .collect::<Vec<u8>>();
        vec.reverse();
        Some(vec)
    }

    // Remove the chars between mark and object
    pub fn remove_from_mark_to_object(&mut self, mark: Mark, object: TextObject) -> Option<Vec<u8>> {
        if let Some(&(mark_idx, _)) = self.marks.get(&mark) {
            let object_index = self.get_object_index(object);

            if let Some((obj_idx, _)) = object_index {
                if mark_idx != obj_idx {
                    let (start, end) = if mark_idx < obj_idx { (mark_idx, obj_idx) } else { (obj_idx, mark_idx) };
                    return self.remove_range(start, end);
                }
            }
        }
        None
    }

    pub fn remove_object(&mut self, object: TextObject) -> Option<Vec<u8>> {
        let object_start = TextObject { kind: object.kind.with_anchor(Anchor::Start), offset: object.offset };
        let object_end = TextObject { kind: object.kind.with_anchor(Anchor::End), offset: object.offset };

        let start = self.get_object_index(object_start);
        let end = self.get_object_index(object_end);

        if let (Some((start_index, _)), Some((end_index, _))) = (start, end) {
            return self.remove_range(start_index, end_index);
        }
        None
    }

    /// Remove the chars at the mark.
    pub fn remove_chars(&mut self, mark: Mark, direction: Direction, num_chars: usize) -> Option<Vec<u8>> {
        let text = &mut self.text;
        if let Some(&(idx, _)) = self.marks.get(&mark) {
            let range = match direction {
                Direction::Left => range(cmp::max(0, idx - num_chars), idx),
                Direction::Right => range(idx, cmp::min(idx + num_chars, text.len())),
                Direction::RightWord(edger) => {
                    let num_words = num_chars;
                    range(idx, get_words(idx, num_words, edger, text).unwrap_or(text.len()))
                }
                Direction::LeftWord(edger) => {
                    let num_words = num_chars;
                    range(get_words_rev(idx, num_words, edger, text).unwrap_or(0), idx)
                }
                _ => unimplemented!()
            };
            let mut transaction = self.log.start(idx);
            let mut vec = range
                .rev()
                .filter_map(|idx| text.remove(idx).map(|ch| (idx, ch)))
                .inspect(|&(idx, ch)| transaction.log(Change::Remove(idx, ch), idx))
                .map(|(_, ch)| ch)
                .collect::<Vec<u8>>();
            vec.reverse();
            Some(vec)
        } else { None }
    }

    /// Insert a char at the mark.
    pub fn insert_char(&mut self, mark: Mark, ch: u8) {
        if let Some(&(idx, _)) = self.marks.get(&mark) {
            self.text.insert(idx, ch);
            let mut transaction = self.log.start(idx);
            transaction.log(Change::Insert(idx, ch), idx);
        }
    }

    /// Redo most recently undone action.
    pub fn redo(&mut self) -> Option<&LogEntry> {
        if let Some(transaction) = self.log.redo() {
            commit(transaction, &mut self.text);
            Some(transaction)
        } else { None }
    }

    /// Undo most recently performed action.
    pub fn undo(&mut self) -> Option<&LogEntry> {
        if let Some(transaction) = self.log.undo() {
            commit(transaction, &mut self.text);
            Some(transaction)
        } else { None }
    }

}

impl WordEdgeMatch {
    /// If c1 -> c2 is the start of a word.
    /// If end of word matching is wanted then pass the chars in reversed.
    fn is_word_edge(&self, c1: &u8, c2: &u8) -> bool {
        // FIXME: unicode support - issue #69
        match (self, *c1 as char, *c2 as char) {
            (_, '\n', '\n') => true, // Blank lines are always counted as a word
            (&WordEdgeMatch::Whitespace, c1, c2) => c1.is_whitespace() && !c2.is_whitespace(),
            (&WordEdgeMatch::Alphabet, c1, c2) if c1.is_whitespace() => !c2.is_whitespace(),
            (&WordEdgeMatch::Alphabet, c1, c2) if is_alpha_or_(c1) => !is_alpha_or_(c2) && !c2.is_whitespace(),
            (&WordEdgeMatch::Alphabet, c1, c2) if !is_alpha_or_(c1) => is_alpha_or_(c2) && !c2.is_whitespace(),
            (&WordEdgeMatch::Alphabet, _, _) => false,
        }
    }
}

fn get_words(mark: usize, n_words: usize, edger: WordEdgeMatch, text: &GapBuffer<u8>) -> Option<usize> {
    range(mark + 1, text.len() - 1)
        .filter(|idx| edger.is_word_edge(&text[*idx - 1], &text[*idx]))
        .take(n_words)
        .last()
}

fn get_words_rev(mark: usize, n_words: usize, edger: WordEdgeMatch, text: &GapBuffer<u8>) -> Option<usize> {
    range(1, mark)
        .rev()
        .filter(|idx| edger.is_word_edge(&text[*idx - 1], &text[*idx]))
        .take(n_words)
        .last()
}

/// Returns the index of the first character of the line the mark is in.
/// Newline prior to mark (EXCLUSIVE) + 1.
fn get_line(mark: usize, text: &GapBuffer<u8>) -> Option<usize> {
    let val = cmp::min(mark, text.len());
    range(0, val + 1).rev().filter(|idx| *idx == 0 || text[*idx - 1] == b'\n')
                           .take(1)
                           .next()
}

/// Returns the index of the newline character at the end of the line mark is in.
/// Newline after mark (INCLUSIVE).
fn get_line_end(mark: usize, text: &GapBuffer<u8>) -> Option<usize> {
    let val = cmp::min(mark, text.len());
    range(val, text.len()+1).filter(|idx| *idx == text.len() || text[*idx] == b'\n')
                            .take(1)
                            .next()
}

/// Performs a transaction on the passed in buffer.
fn commit(transaction: &LogEntry, text: &mut GapBuffer<u8>) {
    for change in transaction.changes.iter() {
        match change {
            &Change::Insert(idx, ch) => {
                text.insert(idx, ch);
            }
            &Change::Remove(idx, _) => {
                text.remove(idx);
            }
        }
    }
}

#[cfg(test)]
mod test {

    use buffer::{Buffer, Direction, Mark};
    use textobject::{TextObject, Offset, Kind, Anchor};

    fn setup_buffer(testcase: &'static str) -> Buffer {
        let mut buffer = Buffer::new();
        buffer.text.extend(testcase.bytes());
        buffer.set_mark(Mark::Cursor(0), 0);
        buffer
    }

    #[test]
    fn move_mark_textobject_char_right() {
        let mut buffer = setup_buffer("Some test content");
        let mark = Mark::Cursor(0);
        let obj = TextObject {
            kind: Kind::Char,
            offset: Offset::Forward(1, mark),
        };

        buffer.set_mark_to_object(mark, obj);
        
        assert_eq!(buffer.marks.get(&mark).unwrap(), &(1, 1));
        assert_eq!(buffer.get_mark_coords(mark).unwrap(), (1, 0));
    }

    #[test]
    fn move_mark_textobject_char_left() {
        let mut buffer = setup_buffer("Some test content");
        let mark = Mark::Cursor(0);
        let obj = TextObject {
            kind: Kind::Char,
            offset: Offset::Backward(1, mark),
        };

        buffer.set_mark(mark, 3);
        buffer.set_mark_to_object(mark, obj);
        
        assert_eq!(buffer.marks.get(&mark).unwrap(), &(2, 2));
        assert_eq!(buffer.get_mark_coords(mark).unwrap(), (2, 0));
    }
    
    #[test]
    fn move_mark_textobject_five_chars_right() {
        let mut buffer = setup_buffer("Some test content");
        let mark = Mark::Cursor(0);
        let obj = TextObject {
            kind: Kind::Char,
            offset: Offset::Forward(5, mark),
        };

        buffer.set_mark_to_object(mark, obj);
        
        assert_eq!(buffer.marks.get(&mark).unwrap(), &(5, 5));
        assert_eq!(buffer.get_mark_coords(mark).unwrap(), (5, 0));
    }

    #[test]
    fn move_mark_textobject_line_down() {
        let mut buffer = setup_buffer("Some test content\nwith new\nlines!");
        let mark = Mark::Cursor(0);
        let obj = TextObject {
            kind: Kind::Line(Anchor::Same),
            offset: Offset::Forward(1, mark),
        };

        buffer.set_mark_to_object(mark, obj);
        
        assert_eq!(buffer.marks.get(&mark).unwrap(), &(18, 0));
        assert_eq!(buffer.get_mark_coords(mark).unwrap(), (0, 1));
    }

    #[test]
    fn move_mark_textobject_line_up() {
        let mut buffer = setup_buffer("Some test content\nnew lines!");
        let mark = Mark::Cursor(0);
        let obj = TextObject {
            kind: Kind::Line(Anchor::Same),
            offset: Offset::Backward(1, mark),
        };

        buffer.set_mark(mark, 18);
        buffer.set_mark_to_object(mark, obj);
        
        assert_eq!(buffer.marks.get(&mark).unwrap(), &(0, 0));
        assert_eq!(buffer.get_mark_coords(mark).unwrap(), (0, 0));
    }

    #[test]
    fn move_mark_textobject_two_lines_down() {
        let mut buffer = setup_buffer("Some test content\nwith new\nlines!");
        let mark = Mark::Cursor(0);
        let obj = TextObject {
            kind: Kind::Line(Anchor::Same),
            offset: Offset::Forward(2, mark),
        };

        buffer.set_mark_to_object(mark, obj);
        
        assert_eq!(buffer.marks.get(&mark).unwrap(), &(27, 0));
        assert_eq!(buffer.get_mark_coords(mark).unwrap(), (0, 2));
    }

    #[test]
    fn move_mark_textobject_line_down_to_shorter_line() {
        let mut buffer = setup_buffer("Some test content\nwith new\nlines!");
        let mark = Mark::Cursor(0);
        let obj = TextObject {
            kind: Kind::Line(Anchor::Same),
            offset: Offset::Forward(1, mark),
        };

        buffer.set_mark(mark, 15);
        buffer.set_mark_to_object(mark, obj);

        assert_eq!(buffer.marks.get(&mark).unwrap(), &(26, 15));
        assert_eq!(buffer.get_mark_coords(mark).unwrap(), (8, 1));

        let obj = TextObject {
            kind: Kind::Line(Anchor::Same),
            offset: Offset::Backward(1, mark),
        };
        buffer.set_mark_to_object(mark, obj);
        
        assert_eq!(buffer.marks.get(&mark).unwrap(), &(15, 15));
        assert_eq!(buffer.get_mark_coords(mark).unwrap(), (15, 0));
    }

    #[test]
    fn move_mark_textobject_two_words_right() {
        let mut buffer = setup_buffer("Some test content\nwith new\nlines!");
        let mark = Mark::Cursor(0);
        let obj = TextObject {
            kind: Kind::Word(Anchor::Start),
            offset: Offset::Forward(2, mark),
        };

        buffer.set_mark_to_object(mark, obj);

        assert_eq!(buffer.marks.get(&mark).unwrap(), &(10, 10));
        assert_eq!(buffer.get_mark_coords(mark).unwrap(), (10, 0));
    }

    #[test]
    fn move_mark_textobject_two_words_left() {
        let mut buffer = setup_buffer("Some test content\nwith new\nlines!");
        let mark = Mark::Cursor(0);
        let obj = TextObject {
            kind: Kind::Word(Anchor::Start),
            offset: Offset::Backward(2, mark),
        };

        buffer.set_mark(mark, 18);
        buffer.set_mark_to_object(mark, obj);

        assert_eq!(buffer.marks.get(&mark).unwrap(), &(5, 5));
        assert_eq!(buffer.get_mark_coords(mark).unwrap(), (5, 0));
    }

    #[test]
    fn move_mark_textobject_move_word_left_at_start_of_buffer() {
        let mut buffer = setup_buffer("Some test content\nwith new\nlines!");
        let mark = Mark::Cursor(0);
        let obj = TextObject {
            kind: Kind::Word(Anchor::Start),
            offset: Offset::Backward(1, mark),
        };

        buffer.set_mark(mark, 5);
        buffer.set_mark_to_object(mark, obj);

        assert_eq!(buffer.marks.get(&mark).unwrap(), &(0, 0));
        assert_eq!(buffer.get_mark_coords(mark).unwrap(), (0, 0));
    }

    #[test]
    fn move_mark_textobject_move_word_right_past_end_of_buffer() {
        let mut buffer = setup_buffer("Some test content\nwith new\nlines!");
        let mark = Mark::Cursor(0);
        let obj = TextObject {
            kind: Kind::Word(Anchor::Start),
            offset: Offset::Forward(8, mark),
        };

        buffer.set_mark(mark, 28);
        buffer.set_mark_to_object(mark, obj);

        assert_eq!(buffer.marks.get(&mark).unwrap(), &(33, 6));
        assert_eq!(buffer.get_mark_coords(mark).unwrap(), (6, 2));
    }

    #[test]
    fn move_mark_textobject_second_word_in_buffer() {
        let mut buffer = setup_buffer("Some test content\nwith new\nlines!");
        let mark = Mark::Cursor(0);
        let obj = TextObject {
            kind: Kind::Word(Anchor::Start),
            offset: Offset::Absolute(2),
        };

        buffer.set_mark(mark, 18);
        buffer.set_mark_to_object(mark, obj);

        assert_eq!(buffer.marks.get(&mark).unwrap(), &(5, 5));
        assert_eq!(buffer.get_mark_coords(mark).unwrap(), (5, 0));
    }

    #[test]
    fn move_mark_textobject_fifth_word_in_buffer() {
        let mut buffer = setup_buffer("Some test content\nwith new\nlines!");
        let mark = Mark::Cursor(0);
        let obj = TextObject {
            kind: Kind::Word(Anchor::Start),
            offset: Offset::Absolute(5),
        };

        buffer.set_mark(mark, 18);
        buffer.set_mark_to_object(mark, obj);

        assert_eq!(buffer.marks.get(&mark).unwrap(), &(23, 5));
        assert_eq!(buffer.get_mark_coords(mark).unwrap(), (5, 1));
    }

    #[test]
    fn move_mark_textobject_second_line_in_buffer() {
        let mut buffer = setup_buffer("Some test content\nwith new\nlines!");
        let mark = Mark::Cursor(0);
        let obj = TextObject {
            kind: Kind::Line(Anchor::Start),
            offset: Offset::Absolute(2),
        };

        buffer.set_mark_to_object(mark, obj);

        assert_eq!(buffer.marks.get(&mark).unwrap(), &(18, 0));
        assert_eq!(buffer.get_mark_coords(mark).unwrap(), (0, 1));
    }

    #[test]
    fn move_mark_textobject_second_char_in_buffer() {
        let mut buffer = setup_buffer("Some test content\nwith new\nlines!");
        let mark = Mark::Cursor(0);
        let obj = TextObject {
            kind: Kind::Char,
            offset: Offset::Absolute(2),
        };

        buffer.set_mark(mark, 18);
        buffer.set_mark_to_object(mark, obj);

        assert_eq!(buffer.marks.get(&mark).unwrap(), &(2, 2));
        assert_eq!(buffer.get_mark_coords(mark).unwrap(), (2, 0));
    }

    #[test]
    fn move_mark_textobject_end_of_line() {
        let mut buffer = setup_buffer("Some test content\nwith new\nlines!");
        let mark = Mark::Cursor(0);
        let obj = TextObject {
            kind: Kind::Line(Anchor::End),
            offset: Offset::Forward(0, mark),
        };

        buffer.set_mark(mark, 19);
        buffer.set_mark_to_object(mark, obj);

        assert_eq!(buffer.marks.get(&mark).unwrap(), &(26, 8));
        assert_eq!(buffer.get_mark_coords(mark).unwrap(), (8, 1));
    }

    #[test]
    fn move_mark_textobject_start_of_line() {
        let mut buffer = setup_buffer("Some test content\nwith new\nlines!");
        let mark = Mark::Cursor(0);
        let obj = TextObject {
            kind: Kind::Line(Anchor::Start),
            offset: Offset::Backward(0, mark),
        };

        buffer.set_mark(mark, 19);
        buffer.set_mark_to_object(mark, obj);

        assert_eq!(buffer.marks.get(&mark).unwrap(), &(18, 0));
        assert_eq!(buffer.get_mark_coords(mark).unwrap(), (0, 1));
    }

    #[test]
    fn test_insert() {
        let mut buffer = setup_buffer("");
        buffer.insert_char(Mark::Cursor(0), b'A');
        assert_eq!(buffer.len(), 2);
        assert_eq!(buffer.lines().next().unwrap(), [b'A']);
    }

    #[test]
    fn test_remove() {
        let mut buffer = setup_buffer("ABCD");
        buffer.remove_chars(Mark::Cursor(0), Direction::Right, 1);

        assert_eq!(buffer.len(), 4);
        assert_eq!(buffer.lines().next().unwrap(), [b'B', b'C', b'D']);
    }

    #[test]
    fn test_set_mark() {
        let mut buffer = setup_buffer("Test");
        buffer.set_mark(Mark::Cursor(0), 2);

        assert_eq!(buffer.get_mark_idx(Mark::Cursor(0)).unwrap(), 2);
    }

    #[test]
    fn test_shift_down() {
        let mut buffer = setup_buffer("Test\nA\nTest");
        buffer.set_mark(Mark::Cursor(0), 2);
        buffer.shift_mark(Mark::Cursor(0), Direction::Down, 2);

        assert_eq!(buffer.get_mark_coords(Mark::Cursor(0)).unwrap(), (2, 2));
    }

    #[test]
    fn test_shift_right() {
        let mut buffer = setup_buffer("Test");
        buffer.shift_mark(Mark::Cursor(0), Direction::Right, 1);

        assert_eq!(buffer.get_mark_idx(Mark::Cursor(0)).unwrap(), 1);
    }

    #[test]
    fn test_shift_up() {
        let mut buffer = setup_buffer("Test\nA\nTest");
        buffer.set_mark(Mark::Cursor(0), 10);
        buffer.shift_mark(Mark::Cursor(0), Direction::Up, 1);

        assert_eq!(buffer.get_mark_coords(Mark::Cursor(0)).unwrap(), (1, 1));
    }

    #[test]
    fn test_shift_left() {
        let mut buffer = setup_buffer("Test");
        buffer.set_mark(Mark::Cursor(0), 2);
        buffer.shift_mark(Mark::Cursor(0), Direction::Left, 1);

        assert_eq!(buffer.get_mark_idx(Mark::Cursor(0)).unwrap(), 1);
    }

    #[test]
    fn test_shift_linestart() {
        let mut buffer = setup_buffer("Test");
        buffer.set_mark(Mark::Cursor(0), 2);
        buffer.shift_mark(Mark::Cursor(0), Direction::LineStart, 0);

        assert_eq!(buffer.get_mark_idx(Mark::Cursor(0)).unwrap(), 0);
    }

    #[test]
    fn test_shift_lineend() {
        let mut buffer = setup_buffer("Test");
        buffer.shift_mark(Mark::Cursor(0), Direction::LineEnd, 0);

        assert_eq!(buffer.get_mark_idx(Mark::Cursor(0)).unwrap(), 4);
    }

    #[test]
    fn test_to_lines() {
        let buffer = setup_buffer("Test\nA\nTest");
        let mut lines = buffer.lines();

        assert_eq!(lines.next().unwrap(), [b'T',b'e',b's',b't',b'\n']);
        assert_eq!(lines.next().unwrap(), [b'A',b'\n']);
        assert_eq!(lines.next().unwrap(), [b'T',b'e',b's',b't']);
    }

    #[test]
    fn test_to_lines_from() {
        let mut buffer = setup_buffer("Test\nA\nTest");
        buffer.set_mark(Mark::Cursor(0), 6);
        let mut lines = buffer.lines_from(Mark::Cursor(0)).unwrap();

        assert_eq!(lines.next().unwrap(), [b'\n']);
        assert_eq!(lines.next().unwrap(), [b'T',b'e',b's',b't']);
    }

    #[test]
    fn test_to_chars() {
        let mut buffer = setup_buffer("Test𐍈Test");
        buffer.set_mark(Mark::Cursor(0), 0);
        let mut chars = buffer.chars();
        assert!(chars.next().unwrap() == 'T');
        assert!(chars.next().unwrap() == 'e');
        assert!(chars.next().unwrap() == 's');
        assert!(chars.next().unwrap() == 't');
        assert!(chars.next().unwrap() == '𐍈');
        assert!(chars.next().unwrap() == 'T');
    }

    #[test]
    fn test_to_chars_from() {
        let mut buffer = setup_buffer("Test𐍈Test");
        buffer.set_mark(Mark::Cursor(0), 2);
        let mut chars = buffer.chars_from(Mark::Cursor(0)).unwrap();
        assert!(chars.next().unwrap() == 's');
        assert!(chars.next().unwrap() == 't');
        assert!(chars.next().unwrap() == '𐍈');
    }

    #[test]
    fn test_to_chars_rev() {
        // 𐍈 encodes as utf8 in 4 bytes... we need a solution for buffer offsets by byte/char
        let mut buffer = setup_buffer("Test𐍈Test");
        buffer.set_mark(Mark::Cursor(0), 8);
        let mut chars = buffer.chars_from(Mark::Cursor(0)).unwrap().reverse();
        assert!(chars.next().unwrap() == 'T');
        assert!(chars.next().unwrap() == '𐍈');
        assert!(chars.next().unwrap() == 't');
    }

    #[test]
    fn move_from_final_position() {
        let mut buffer = setup_buffer("Test");
        buffer.set_mark(Mark::Cursor(0), 4);
        buffer.shift_mark(Mark::Cursor(0), Direction::Left, 1);

        assert_eq!(buffer.get_mark_idx(Mark::Cursor(0)).unwrap(), 3);
    }

}
