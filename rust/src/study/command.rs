//! Private study command editing and parsing.
//!
//! Commands are renderer controls, not task workloads. This module therefore
//! has no dependency on phases, task registration, scheduling, or terminal
//! drawing.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StudyCommand {
    Exit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum EditAction {
    Insert(char),
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
    Clear,
    Submit,
}

#[derive(Debug, Eq, PartialEq)]
pub(super) enum CommandSubmission {
    Empty,
    Parsed(StudyCommand),
    Unknown(String),
}

#[derive(Default)]
pub(super) struct CommandInput {
    characters: Vec<char>,
    cursor: usize,
}

impl CommandInput {
    pub(super) fn edit(&mut self, action: EditAction) -> Option<CommandSubmission> {
        match action {
            EditAction::Insert(character) => {
                self.characters.insert(self.cursor, character);
                self.cursor += 1;
            }
            EditAction::Backspace if self.cursor > 0 => {
                self.cursor -= 1;
                self.characters.remove(self.cursor);
            }
            EditAction::Delete if self.cursor < self.characters.len() => {
                self.characters.remove(self.cursor);
            }
            EditAction::Left => self.cursor = self.cursor.saturating_sub(1),
            EditAction::Right => self.cursor = (self.cursor + 1).min(self.characters.len()),
            EditAction::Home => self.cursor = 0,
            EditAction::End => self.cursor = self.characters.len(),
            EditAction::Clear => self.clear(),
            EditAction::Submit => {
                let command = self.text();
                self.clear();
                return Some(parse(&command));
            }
            _ => {}
        }
        None
    }

    pub(super) fn text(&self) -> String {
        self.characters.iter().collect()
    }

    pub(super) fn cursor(&self) -> usize {
        self.cursor
    }

    pub(super) fn clear(&mut self) {
        self.characters.clear();
        self.cursor = 0;
    }
}

fn parse(input: &str) -> CommandSubmission {
    match input.trim() {
        "" => CommandSubmission::Empty,
        "exit" => CommandSubmission::Parsed(StudyCommand::Exit),
        unknown => CommandSubmission::Unknown(unknown.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_supports_insertion_navigation_and_submission() {
        let mut command = CommandInput::default();
        for character in ['e', 'x', 't'] {
            assert_eq!(command.edit(EditAction::Insert(character)), None);
        }
        command.edit(EditAction::Left);
        command.edit(EditAction::Insert('i'));
        assert_eq!(command.text(), "exit");
        assert_eq!(command.cursor(), 3);
        assert_eq!(
            command.edit(EditAction::Submit),
            Some(CommandSubmission::Parsed(StudyCommand::Exit))
        );
        assert_eq!(command.text(), "");
    }

    #[test]
    fn parser_accepts_only_the_renderer_command_contract() {
        assert_eq!(
            parse(" exit \t"),
            CommandSubmission::Parsed(StudyCommand::Exit)
        );
        assert_eq!(parse(""), CommandSubmission::Empty);
        assert_eq!(
            parse("status"),
            CommandSubmission::Unknown("status".to_owned())
        );
        assert_eq!(parse("EXIT"), CommandSubmission::Unknown("EXIT".to_owned()));
    }
}
