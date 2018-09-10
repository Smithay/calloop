use std::cell::RefCell;
use std::rc::Rc;

use mio::Token;

use sources::EventDispatcher;

pub(crate) struct SourceList<Data> {
    sources: Vec<Option<Rc<RefCell<EventDispatcher<Data>>>>>,
}

impl<Data> SourceList<Data> {
    pub(crate) fn new() -> SourceList<Data> {
        SourceList {
            sources: Vec::new(),
        }
    }

    pub(crate) fn get_dispatcher(
        &self,
        token: Token,
    ) -> Option<Rc<RefCell<EventDispatcher<Data>>>> {
        match self.sources.get(token.0) {
            Some(&Some(ref dispatcher)) => Some(dispatcher.clone()),
            _ => None,
        }
    }

    pub(crate) fn add_source(&mut self, dispatcher: Rc<RefCell<EventDispatcher<Data>>>) -> Token {
        let free_id = self.sources.iter().position(Option::is_none);
        if let Some(id) = free_id {
            self.sources[id] = Some(dispatcher);
            Token(id)
        } else {
            self.sources.push(Some(dispatcher));
            Token(self.sources.len() - 1)
        }
    }

    pub(crate) fn del_source(&mut self, token: Token) {
        self.sources[token.0] = None;
    }
}

pub(crate) trait ErasedList {
    fn del_source(&mut self, token: Token);
}

impl<Data> ErasedList for SourceList<Data> {
    fn del_source(&mut self, token: Token) {
        self.del_source(token);
    }
}
