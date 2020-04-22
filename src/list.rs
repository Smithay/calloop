use std::rc::Rc;

use crate::Token;

use crate::sources::EventDispatcher;

pub(crate) struct SourceList<Data> {
    sources: Vec<Option<Rc<dyn EventDispatcher<Data>>>>,
}

impl<Data> SourceList<Data> {
    pub(crate) fn new() -> SourceList<Data> {
        SourceList {
            sources: Vec::new(),
        }
    }

    pub(crate) fn contains(&self, token: Token) -> bool {
        self.sources
            .get(token.id as usize)
            .map(Option::is_some)
            .unwrap_or(false)
    }

    pub(crate) fn get_dispatcher(&self, token: Token) -> Option<Rc<dyn EventDispatcher<Data>>> {
        match self.sources.get(token.id as usize) {
            Some(&Some(ref dispatcher)) => Some(dispatcher.clone()),
            _ => None,
        }
    }

    pub(crate) fn add_source(&mut self, dispatcher: Rc<dyn EventDispatcher<Data>>) -> Token {
        let free_id = self.sources.iter().position(Option::is_none);
        if let Some(id) = free_id {
            self.sources[id] = Some(dispatcher);
            Token {
                id: id as u32,
                sub_id: 0,
            }
        } else {
            self.sources.push(Some(dispatcher));
            Token {
                id: (self.sources.len() - 1) as u32,
                sub_id: 0,
            }
        }
    }

    // this method returns the removed dispatcher to ensure it is not dropped
    // while the refcell containing the list is borrowed, as dropping a dispatcher
    // can trigger the removal of an other source
    pub(crate) fn del_source(&mut self, token: Token) -> Option<Rc<dyn EventDispatcher<Data>>> {
        ::std::mem::replace(&mut self.sources[token.id as usize], None)
    }
}
