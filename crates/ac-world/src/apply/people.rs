use ac_net::wire::Reader;

use crate::{allegiance, housing, parse_fellow, social, Applied, Confirmation, Fellowship, World};

impl World {
    pub(super) fn apply_fellowship_full_update(&mut self, rest: &[u8]) -> Applied {
        let mut r = Reader::new(rest);
        let parsed = (|| {
            let n = r.u16().ok()? as usize;
            let _buckets = r.u16().ok()?;
            let mut members = Vec::with_capacity(n.min(16));
            for _ in 0..n {
                members.push(parse_fellow(&mut r)?);
            }
            let name = r.string16().ok()?;
            let leader = r.u32().ok()?;
            let share_xp = r.u32().ok()? != 0;
            let even_share = r.u32().ok()? != 0;
            let open = r.u32().ok()? != 0;
            let locked = r.u32().ok()? != 0;
            Some(Fellowship {
                name,
                leader,
                share_xp,
                even_share,
                open,
                locked,
                members,
            })
        })();
        match parsed {
            Some(f) => {
                self.fellowship = Some(f);
                self.generation += 1;
                Applied::Fellowship
            }
            None => Applied::Failed,
        }
    }

    pub(super) fn apply_fellow_update(&mut self, rest: &[u8]) -> Applied {
        let mut r = Reader::new(rest);
        match (parse_fellow(&mut r), self.fellowship.as_mut()) {
            (Some(f), Some(fs)) => {
                match fs.members.iter_mut().find(|m| m.guid == f.guid) {
                    Some(m) => *m = f,
                    None => fs.members.push(f),
                }
                Applied::Fellowship
            }
            _ => Applied::Ignored,
        }
    }

    pub(super) fn apply_fellowship_disband(&mut self) -> Applied {
        self.fellowship = None;
        self.generation += 1;
        Applied::Fellowship
    }

    pub(super) fn apply_fellow_gone(&mut self, rest: &[u8]) -> Applied {
        let who = Reader::new(rest).u32().unwrap_or(0);
        if Some(who) == self.player_guid {
            self.fellowship = None;
        } else if let Some(fs) = self.fellowship.as_mut() {
            fs.members.retain(|m| m.guid != who);
        }
        self.generation += 1;
        Applied::Fellowship
    }

    pub(super) fn apply_confirmation_request(&mut self, rest: &[u8]) -> Applied {
        let mut r = Reader::new(rest);
        match (r.u32(), r.u32(), r.string16()) {
            (Ok(kind), Ok(context), Ok(text)) => {
                self.confirmations.push(Confirmation {
                    kind,
                    context,
                    text,
                });
                Applied::Confirmation
            }
            _ => Applied::Failed,
        }
    }

    pub(super) fn apply_allegiance_update(&mut self, rest: &[u8]) -> Applied {
        let mut r = Reader::new(rest);
        let me = self.player_guid.unwrap_or(0);
        match r
            .u32()
            .ok()
            .and_then(|rank| allegiance::parse_profile(&mut r, me, rank))
        {
            Some(a) => {
                self.allegiance = Some(a);
                Applied::Allegiance
            }
            None => Applied::Failed,
        }
    }

    pub(super) fn apply_allegiance_info(&mut self, rest: &[u8]) -> Applied {
        let mut r = Reader::new(rest);
        match r
            .u32()
            .ok()
            .and_then(|who| Some((who, allegiance::parse_profile(&mut r, who, 0)?)))
        {
            Some(info) => {
                self.allegiance_info = Some(info);
                Applied::Allegiance
            }
            None => Applied::Failed,
        }
    }

    pub(super) fn apply_allegiance_login(&mut self, rest: &[u8]) -> Applied {
        let mut r = Reader::new(rest);
        if let (Ok(who), Ok(on)) = (r.u32(), r.u32()) {
            if let Some(m) = self.allegiance.as_mut().and_then(|a| a.member_mut(who)) {
                m.online = on != 0;
            }
        }
        Applied::Allegiance
    }

    pub(super) fn apply_house_profile(&mut self, rest: &[u8]) -> Applied {
        match housing::parse_profile(rest) {
            Some(p) => {
                self.house_profile = Some(p);
                self.house_profile_seq += 1;
                Applied::House
            }
            None => Applied::Failed,
        }
    }

    pub(super) fn apply_house_data(&mut self, rest: &[u8]) -> Applied {
        match housing::parse_data(rest) {
            Some(d) => {
                self.house = Some(Some(d));
                Applied::House
            }
            None => Applied::Failed,
        }
    }

    pub(super) fn apply_house_status(&mut self) -> Applied {
        // The answer to HouseQuery without a house.
        self.house = Some(None);
        self.house_access = None;
        Applied::House
    }

    pub(super) fn apply_update_har(&mut self, rest: &[u8]) -> Applied {
        match housing::parse_access(rest) {
            Some(a) => {
                self.house_access = Some(a);
                Applied::House
            }
            None => Applied::Failed,
        }
    }

    pub(super) fn apply_rent_time(&mut self, rest: &[u8]) -> Applied {
        if let (Ok(t), Some(Some(h))) = (Reader::new(rest).u32(), self.house.as_mut()) {
            h.rent_time = t;
        }
        Applied::House
    }

    pub(super) fn apply_rent_payment(&mut self, rest: &[u8]) -> Applied {
        let mut r = Reader::new(rest);
        if let (Some(rent), Some(Some(h))) =
            (housing::read_payments_pub(&mut r), self.house.as_mut())
        {
            h.rent = rent;
        }
        Applied::House
    }

    pub(super) fn apply_friends_update(&mut self, rest: &[u8]) -> Applied {
        match social::parse_friends(rest) {
            Some(u) => {
                match u.kind {
                    0 => self.friends = u.friends,
                    2 => {
                        for f in &u.friends {
                            self.friends.retain(|x| x.guid != f.guid);
                        }
                    }
                    _ => {
                        for f in u.friends {
                            match self.friends.iter_mut().find(|x| x.guid == f.guid) {
                                Some(x) => {
                                    x.online = f.online;
                                    if !f.name.is_empty() {
                                        x.name = f.name;
                                    }
                                }
                                None => self.friends.push(f),
                            }
                        }
                    }
                }
                Applied::Social
            }
            None => Applied::Failed,
        }
    }

    pub(super) fn apply_character_title(&mut self, rest: &[u8]) -> Applied {
        match social::Titles::parse(rest) {
            Some(t) => {
                self.titles = t;
                Applied::Social
            }
            None => Applied::Failed,
        }
    }

    pub(super) fn apply_update_title(&mut self, rest: &[u8]) -> Applied {
        if self.titles.apply_update(rest) {
            Applied::Social
        } else {
            Applied::Failed
        }
    }

    pub(super) fn apply_squelch_db(&mut self, rest: &[u8]) -> Applied {
        match social::parse_squelches(rest) {
            Some(sq) => {
                self.squelches = sq;
                Applied::Social
            }
            None => Applied::Failed,
        }
    }

    pub(super) fn apply_confirmation_done(&mut self, rest: &[u8]) -> Applied {
        let mut r = Reader::new(rest);
        if let (Ok(kind), Ok(context)) = (r.u32(), r.u32()) {
            self.confirmations
                .retain(|c| !(c.kind == kind && c.context == context));
        }
        Applied::Confirmation
    }
}
