use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Link {
    Detached,
    Attached(Duration),
}

impl Default for Link {
    fn default() -> Self {
        Link::Attached(Duration::from_secs(1))
    }
}

impl Link {
    pub fn attach(&mut self, mut duration: Duration) -> Option<Duration> {
        match self {
            Link::Detached => {
                *self = Link::Attached(duration);
                None
            }
            Link::Attached(old_duration) => {
                std::mem::swap(old_duration, &mut duration);
                Some(duration)
            }
        }
    }

    pub fn detach(&mut self) -> Option<Duration> {
        match self {
            Link::Detached => {
                *self = Link::Detached;
                None
            }
            Link::Attached(_) => {
                let mut link = Link::Detached;
                std::mem::swap(self, &mut link);
                match link {
                    Link::Attached(duration) => Some(duration),
                    Link::Detached => unreachable!(),
                }
            }
        }
    }
}