# The autoplay agent

How a character decides what to do, why it is built the way it is, and
what it is being moved towards.

See also: [architecture.md](architecture.md) (the crate map),
[multi-session.md](multi-session.md) (many characters at once).

## What this has to be good at

A character is an agent in a real-time game, ticked at 20 Hz, with a
dozen or more of it running in one process. That puts four demands on
the design, and they pull in different directions:

- **Reflexes must be instant and cheap.** A spell in the air, health at
  40%, the player grabbing the keys: these preempt everything and must
  cost nothing to check, because the cost is paid by every session on
  every tick.
- **Plans must survive interruption.** A trip to town is minutes long,
  crosses landblocks, and can be broken off by a fight, a death or a
  disconnection. It has to resume, not restart.
- **Failure is the normal case.** Almost nothing a character tries is
  guaranteed: the counter will not take the item, the corpse will not
  open, the doorway will not admit it, the purse is empty, the pack is
  full, the quest is on cooldown. Most of the work is deciding what to
  do when told no.
- **A party must agree without negotiating.** Every session works out
  the group's mode from the same roster and must reach the same answer,
  because there is no authority to arbitrate and no time to vote.

## What it is now

Three things that grew separately.

**A flattened behaviour tree.** `Client::tick_autoplay` is a fixed
priority chain -- dodge, survive, recover, academy, buff, vitals, loot,
follow, team, salvage, fight, buff again, follow again, explore,
grow --
where each step returns whether it claimed the tick. That is exactly a
behaviour tree's root selector, written out by hand. It works, and the
order is genuinely load-bearing, but the order is control flow rather
than data: nothing can read it, show it, or reason about it.

Tidying the pack is not in the chain. Pouring one carried stack into
another is made by the server on the spot, with no walk and no
animation, so it costs no tick and runs as housekeeping. As a step it
never won one -- a character that fights, loots and walks all afternoon
has no quiet tick to give it -- and the pack filled up with part stacks
while tidying waited its turn.

Spending experience moved out for the same reason. It was the first
thing the grow step did, and grow is the last goal: reached only when
nothing else wants the tick. Something always did. A character granted a
hundred billion experience on the local server spent none of it in three
minutes, because exploring claimed every tick and always had another
room to walk to. A raise is one message, and the server takes it with no
busy check, no animation and no movement -- it checks the stat and the
pool, spends, and answers -- so it runs as housekeeping too, in the
middle of a walk or a fight, still paced to one message at a time. A
message can carry many ranks of one stat: a large pool goes out as the
ranks buying one at a time would have given each stat, a message a stat,
so it is spread as before and spent in seconds rather than days. The
best buy is bought or saved for, never passed over for a cheaper rank
that happens to fit: the kills after a large pool went only to the
cheapest skills, and the ones a character fights with stopped growing.
A rank that raises a maximum (Health, Stamina, Mana, or Endurance and
Self, which they are built from) waits for the fight to be over, and its
share of the pool waits with it. The fight under way is fought with what
is left, which that rank leaves where it was, and the fraction left
drops with it: a character with a large pool bought Health between
swings until it healed, mid-fight, health it had never lost. A raise the
server has not answered keeps its share too, and is given up on only
once it has had time to arrive: a server that answers late, or a round
of nine headless characters that takes seconds, is not a refusal.

Watching the ground runs before even the reflexes, for the same reason
and one more. The clocks it keeps -- how long a spot has had nothing on
it, and when each body on it first came into sight -- are read off the
world every tick, not off what the character happens to be doing: the
hunting step that acts on the first is the last goal in the table, so a
character with a body to open never reaches it, and the second used to
be wound inside the looting, which a character standing off from a body
never runs at all. A clock only wound where it is read stands still
exactly when it is most needed. Nine characters queueing at one corpse
restarted the quiet clock every couple of seconds and never once walked
the two hundred metres they had been given; and a body nobody had noted
stayed for ever "newly fallen", so the claim tie-break over it never
expired. Housekeeping alone was not enough for either: a tick claimed by
a dodge, a heal, a death or the Academy returns before the housekeeping
table is reached.

A reflex is paid for by every tick it claims, so a reflex has to be
worth the tick. Dodging sits second in the chain, ahead of the healing,
because a spell already in the air cannot be argued with -- and it took
up every projectile in view without asking whose it was. Nine
characters shooting the same creature from a metre apart aimed a fifth
of one run's dodges at each other, and each of those held the legs for
six hundred milliseconds and claimed the tick for a bolt that could not
have hurt anybody. The place in the chain was right; what was missing
was the price. A projectile that cannot hurt the character is still
worth stepping out of the way of -- the server destroys it on us and
the fellow loses his spell -- but only when the moment is going spare
(`dodge`).

**A group state machine.** `logistics` is a proper FSM over
`Hunting | Restocking{Shopping, HandOver, Away, HandOut}`, computed by
every session from the shared roster. This part is right and is not
changing: determinism is the whole point of it.

**Sixty-two fields of state.** `Autoplay` carries 62 fields and
`growth::State` another 32. Many are legitimate memory. A growing
number are not: `too_heavy`, `run_was_futile`, `stopped_in_town`,
`handed_over`, `give_tries`, `take_tries`, `refused_kinds`, `shelved`,
`walking_to`, `reaching`, `unsellable`, `skip_vendors`, `held_back`.

Each of those is the scar of one bug. Every one of them encodes the
same thing in a slightly different way: *something was refused, and
here is how long to wait before asking again.*

## The actual problem

Every failure found in a day of live testing was one of two kinds:

- **An unmodelled precondition.** Buying without checking the weight it
  would add. Offering a vendor an item it will never take. Walking to a
  counter behind a door the character cannot open. Asking for a
  component no shop in the world sells.
- **An unhandled outcome.** A give of part of a stack that the server
  drops without a word, asked 496 times. A corpse that will not open,
  asked until it rotted. An item refused for the day, asked on every
  corpse after.

Both have the same root: **an action returns `bool`.** True means it
did something. False means... it did not, this tick, for a reason the
caller cannot see and therefore cannot act on. So "keeps trying for
ever" is not a bug that was written; it is the default that falls out
of the type. Every fix has been a new flag bolted on beside the call
site, which is why there are sixty-two of them.

This is the thing to fix, and it is independent of any larger
restructuring.

## Where it is going

Four layers, because the four demands above are genuinely different
problems and one paradigm serves none of them well.

### 0. Reflexes -- fixed priority

Dodge, survive, recover, hand back to the player. An ordered list, no
scoring, no planning, O(1). This is what the current chain does well and
it stays as it is.

### 1. Goal selection -- utility

Score the standing goals each tick -- Fight, Loot, Restock, Buff,
Follow, Salvage, Idle -- from world state, and take the best.

Static priority is what makes the present code brittle: the order of
the chain silently decides a hundred questions, and every "except when"
becomes another branch. Utility says the same things out loud and in
one place. It is already being hand-written as constants: `CORPSE_URGENT`
is "looting outscores fighting when the body is nearly gone",
`go_at: 0.35` is "restocking outscores hunting below a third of stock".
Those are utility curves with the arithmetic inlined.

Scoring must stay cheap: a handful of arithmetic per goal, no
allocation, because it runs per session per tick.

### 2. Plans -- behaviour trees, and one planner

Each goal expands into a small behaviour tree: a sequence of steps with
guards, resumable, interruptible. Most goals are honestly a fixed
sequence and a tree says so plainly.

**Restocking is the exception and should be planned.** It is the one
goal with interacting preconditions over a shared resource pool --
purse, burden, slots, what the shop stocks, what it will buy, whether
the door opens, whether the place can be reached -- and it is where
every bug has been. A small domain planner over those preconditions
would have *derived* "cannot buy because too heavy, therefore sell
first" instead of it being patched in after the fact. Not general GOAP:
a handful of actions with declared preconditions and effects, searched
over a tiny state, replanned only when something changes.

### 3. The group -- keep the FSM

Unchanged. Small, deterministic, and its determinism is the point.

### Why not one paradigm

- **Pure FSM**: state explosion. Every new concern is another flag or
  another state, which is the road already travelled.
- **Pure behaviour tree**: leaves goal selection as static priority,
  which is today's problem.
- **Pure GOAP**: a replan per agent per tick is the wrong cost across a
  dozen sessions, it is hard to answer "why did it do that", and
  reflexes must never be planned at all.
- **Pure utility**: no good account of multi-step plans, which is most
  of what a trip to town is.

## The migration

Four stages, each shippable on its own, in this order. Nothing here is
a rewrite; the behaviour at each step is meant to be identical except
where a bug is being fixed.

### Stage 1 -- typed outcomes

Replace `bool` with an outcome that says what happened:

```rust
pub enum Did {
    /// Claimed the tick and is working.
    Acting,
    /// Finished; let something else have the tick.
    Done,
    /// Cannot proceed yet, for a reason that will pass by itself.
    Waiting(Because),
    /// Cannot proceed, and asking again soon will not help.
    Blocked(Because),
    /// Will never work. Do not ask again.
    Refused(Because),
}
```

`Because` carries a short reason and, where the server gave one, the
weenie error behind it. One retry policy reads that: `Waiting` retries
freely, `Blocked` backs off, `Refused` stops. That single change
subsumes `give_tries`, `take_tries`, `refused_kinds`, `shelved`,
`unsellable`, `run_was_futile`, `too_heavy` and `skip_vendors`, and it
makes every one of them visible in the log and the UI instead of
invisible in a boolean.

Done first because it is mechanical, testable, pays for itself
immediately, and makes the later stages cheaper.

**Done**: the looting waits (`shelved`, `refused_kinds`, `take_tries`),
the selling ones (`unsellable`, `skip_vendors`) and the hand-over
(`give_tries`). Six flags and five constants replaced by one policy.

**Deliberately not done here**: `run_was_futile`, `too_heavy` and
`stopped_in_town`. They look like the others but are not. All three are
read by `Supplies::broke`, which is how a character tells the *party*
it is done shopping and should be carried on without -- so they are
goal-layer state, not retry bookkeeping, and forcing them into a
`Patience` would hide that. They become one honest "this character
cannot restock, and here is why" in stage 3.

### Stage 2 -- the chain becomes data

**Done.** `steps::STEPS`: the same order, each entry carrying its name
and why it sits where it does, and marked reflex or goal. Tests hold
that healing comes before hunting, that looting comes before the shops,
and that no reflex sits below a goal.

### Stage 3 -- utility at the root

**Done.** Goals are scored; one with no opinion takes its place in the
table, so nothing moved until a curve was written on purpose.

The first curve: a body is worth more every second it waits and each
one still on the floor adds to that, so past about a third of its life
a body outranks starting another fight. A character that only broke off
for a corpse about to rot never went back for the older ones, because
in a busy dungeon there is always another fight.

### Stage 4 -- the errand planner

**Done.** `errand::plan` walks the resources forward -- sell, cash, buy,
convert -- and each act takes from the pool what it costs and gives
back what it yields. "Cannot buy because too laden, therefore sell
first" is not written down: it falls out of selling happening before
buying and of weight being counted.

It is arithmetic over a plain state, so it runs before the character
leaves as well as when it arrives, which is how a trip is known to be
worth making.

## What is left

- The three goal-layer flags stage one left alone: `run_was_futile`,
  `too_heavy`, `stopped_in_town`. They belong with `Supplies::broke` as
  one "cannot restock, and why" that the party can read.
- The steps still answer `Did::Acting` or `Did::Done` and nothing else.
  Each one that learns to say why it stood aside makes the log and the
  autoplay panel better; none of them has to.
- Reachability is not yet a precondition anywhere. The planner knows
  what a counter stocks and what the character can pay and lift; it
  does not know that the counter is up a flight of stairs the
  navigation graph has no path to.

## What the server will and will not tell a character

Two things that cost a day each to find, because neither looks like a
bug from inside the client.

**A dungeon is described a room at a time.** The server sends the
creatures standing in the cell the character is in and in the cells
that one can see, and nothing else. Holtburg Dungeon holds fifty-eight
creatures across seventy-four rooms; the room its portal drops you in
holds two monster generators, which are server-side and are described
to nobody. So a character that arrives and waits sees an empty dungeon
and reports one. The cure is `crate::explore`: walk the rooms, one
doorway at a time, aiming a couple of paces *past* each threshold --
stop on the threshold and the server goes on describing the room
behind.

**The world you leave stays in the object table.** ACE holds an object
that has dropped out of sight for twenty-five seconds before it sends
the delete, so for those seconds a character that has stepped through a
portal still has the town it left, and picks a creature thirty
kilometres behind it to go and fight. `World::arrived_in` sets aside
anything out of sight when the landblock changes -- by distance, not by
landblock, because outdoor landblocks are seen across their borders and
walking into the next one must not throw away the corpse just made.

Set aside, not forgotten. A teleport does not clear what ACE thinks the
client knows: an object out of sight waits in a queue for its delete,
and one back in sight before the twenty-five seconds are up leaves the
queue and is never described again. A character killed just inside the
Holtburg Dungeon rose at the lifestone next door inside that time, and
the portal it had thrown away never came back; it stood beside the
mouth for good. What is set aside returns when the character comes
within sight of it or the server speaks of it, and goes for good when
the server deletes it.

## Getting there is four questions, and height is in all of them

A character sent to Asenala, who keeps a shop on the upper floor of a
house in Holtburg, walked a hundred and forty metres, stopped six
millimetres from her on the map and three metres below her on the
ground floor, and stood there for good. Five separate things had to be
right before it could climb the stairs, and four of them were the same
mistake: a place is a point in three dimensions, and every one of these
was judging it on the flat.

- **Is the goal's height to be believed?** Both planners grounded a
  goal onto the terrain under it unless told otherwise, and what told
  them was a cell id that the steering fills in with a *landblock* --
  whose low word is zero, which reads as "outdoors". Neither the
  block's graph nor the neighbourhood planner ever saw the real
  height. They ask the geometry now (`CollisionWorld::in_known_cell`),
  which is not something a caller can get wrong.
- **Have we arrived?** `flat.length() > stop`. Standing under someone
  is not standing with them: arrival counts height now, to within a
  doorsill.
- **Have we reached this waypoint?** The same again, one level down,
  and worse: a waypoint at the top of a staircase is a pace away on the
  map, so the route was thrown away a waypoint at a time and the
  character left aiming at a point above its own head.
- **How far may one frame carry us?** Building a chunk of the
  navigation graph takes a couple of hundred milliseconds and the walk
  that follows is charged the whole of it at running speed -- three
  metres in one step, past the waypoint and out the far side, turned
  round by the next frame. At the foot of a staircase that reads as a
  character crossing and re-crossing the bottom step for ever. A frame
  may not carry us past what we are walking to.

And one that was not about height at all: **a wedge has to let go**.
The rule that stops a character leaning on a wall returned before the
line that cleared its own counter, so the first doorframe a character
brushed froze it for the rest of the session -- for that errand and
every errand after it. It now clears the moment the character is asked
to go somewhere else, and otherwise rests a second and tries again,
because doors open and whatever was leaned on walks away.

## Rules that hold whatever the structure

These are settled and are not up for redesign by a later stage.

- **Never sell** what is tinkered, inscribed, equipped, retained, or
  flagged unsellable, whatever any rule or profile says. A policy may
  not override it.
- **Nothing is given up on for good** except by an explicit `Refused`.
  A session can run for days; a daily limit is a wait, not a grudge.
- **The player's hands win.** Any manual input stops the character
  steering itself unless autoplay is on.
- **A profile is shared, live.** A rule switched off is off for every
  character reading that profile, at once.
