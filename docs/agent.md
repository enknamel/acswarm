# The autoplay agent

How a character decides what to do, and why it is built that way. The
order of the steps lives in `crates/ac-client/src/steps.rs`; the code
map is `crates/ac-client/CLAUDE.md`.

See also: [CLAUDE.md](../CLAUDE.md) (the crate map),
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

## How it is built

Four layers, because the four demands above are different problems and
one paradigm serves none of them well.

**Reflexes: fixed priority.** Healing, stepping out of a projectile's
way, coming back from a death, the Training Academy, a buff about to
lapse, vitals. The reflex rows of `steps::STEPS`, run in order with no
scoring. A reflex is paid for by every tick it claims, so it has to be
worth the tick: a projectile that cannot hurt the character is stepped
out of the way of only when the moment is going spare (`dodge`).

**Goals: utility.** The goal rows of `steps::STEPS` are scored from the
world each tick and the best one that acts takes the tick. A goal with
no curve of its own is worth its place in the table, so the order holds
until a curve is written on purpose. Looting has one: a body is worth
more every second it waits and each body still on the floor adds to it,
so past about a third of its life a body outranks starting another
fight. Scoring stays cheap, a few sums per goal and no allocation,
because it runs per session per tick.

**Housekeeping: every tick, never claiming it.** `steps::HOUSEKEEPING`
holds what the server does on the spot with no walk, no animation and
no busy check: pouring stacks together, taking up a weapon, restocking a
quiver, spending experience. As goals they never won a tick, because a
character that fights, loots and walks is never idle. The clocks read
off the ground -- how long a spot has been quiet, when each body first
came into sight -- are wound before even the reflexes
(`autoplay_watch_the_ground`), because a clock wound only where it is
read stands still exactly when it is needed.

Spending experience is paced to one message at a time, and a message
can carry many ranks of one stat. The best buy is bought or saved for,
never passed over for a cheaper rank that fits. A rank that raises a
maximum (Health, Stamina, Mana, or Endurance and Self) waits for the
fight to be over and keeps its share of the pool meanwhile, and a raise
the server has not answered keeps its share until it has had time to
arrive: a late answer is not a refusal.

**Plans: small sequences, and one planner.** Each goal is a short
resumable sequence with guards. The town run is the exception and is
planned: it has interacting preconditions over a shared pool -- purse,
burden, slots, what the counter stocks and will buy -- and `ac_vendor`
walks the resources forward (sell, cash, buy, convert), each act taking
what it costs and giving what it yields. "Sell first, because too laden
to buy" falls out of the arithmetic rather than being patched in. It is
arithmetic over a plain state, so a trip is judged before the character
leaves as well as at the counter.

**The group: a state machine, with a plan on top.** `logistics` is an
FSM over `Hunting | Restocking{Shopping, HandOver, Away, HandOut}`,
computed by every session from the shared roster, and determinism is
the point of it: there is no authority to arbitrate. The leader also
plans (`plan`): who fights what, spread by who is being attacked and by
distance with focus fire kept for hard targets; whose turn each body
is; and whom it waits for before moving the party on -- a follower
still fighting or left behind, and the one it dealt a body to. The plan
goes out on the board, an order is obeyed only while it is fresh, and a
session with no fresh order falls back to its own rules, so a plan that
never arrives stops nobody fighting. The plan reads the roster and never
drives it: it does not choose the leader, found the fellowship or start
the walk to town, and every decision in it is a pure function a test
can run without a clock or a server.

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

## Outcomes, not booleans

Almost nothing a character tries is guaranteed, and a step that answers
only "acted or not" leaves its caller nothing to act on: "keeps trying
for ever" is the default that falls out of a `bool`. So steps answer
`did::Did` -- `Acting`, `Done`, `Waiting(Because)`, `Blocked(Because)`
or `Refused(Because)` -- where `Because` carries a short reason and,
when the server gave one, the weenie error behind it. One retry policy,
`did::Patience`, reads it: waiting retries freely, blocked backs off,
refused stops. A new wait on a refusal belongs there, not in a flag
beside the call site.

## What is left

- `run_was_futile`, `too_heavy` and `stopped_in_town` look like retry
  bookkeeping but are goal-layer state: they tell the party this
  character cannot restock. They belong together as one "cannot
  restock, and here is why" that the party can read.
- Most steps still answer only `Did::Acting` or `Did::Done`. Each one
  that learns to say why it stood aside makes the log and the autoplay
  panel better; none of them has to.
- Reachability is not yet a precondition of the town-run planner. It
  knows what a counter stocks and what the character can pay and lift,
  not that the counter is up a flight of stairs the navigation graph
  has no path to.

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
behind. Past it *straight through the wall* (`CellScene::doorway_normals`,
the portal polygon's normal), on the far side from where we stand, or
the way we face when we stand on the sill: aimed along the line we
approached on, a character coming at a door from the corner of the
room before it was sent into the corridor's side wall, and aimed from
the sill itself it was sent nowhere and stood ten seconds for the room
to be written off. And on a floor: the Holtburg Dungeon's corridors run
diagonally from their doors, so an aim with nothing to stand on is
turned forty-five degrees either way, then drawn in (`aim_on_floor`).

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

## What the server says in words

Not every refusal comes as a code. ACE answers a good many in plain
chat -- a transient string (`GameEventCommunicationTransientString`) or
a Broadcast line (`GameMessageSystemChat`) -- and says nothing else:
the game event that follows, when there is one, carries no reason, and
often there is none at all. A client that reads only the codes sees a
request go quiet, waits out a clock, and asks again for ever. Two mates
were invited into a fellowship ten times each because "{Name} is busy."
was never read; a body was opened thirty times for a take that "Unable
to put {item} into container" had already answered; a body someone else
had open was asked for on a clock while "The Corpse of X is already in
use by someone else!" sat in the chat log.

So there is one place the words are read: `ac_agent::refusals::refused`
matches every line of server chat against a table quoted from the ACE
source, file and line, and `refusals::answer` decides once, by kind,
what to do -- ask again after a doubling wait, ask again after the same
short wait, or never. `Client::hear_refusal` (`crates/ac-client/src/refused.rs`)
hands each refusal to the system that was waiting on it. No system
matches English of its own; the tests for the wording live with the
table, in the server's exact words with a name substituted.

**Read and acted on** (the request autoplay was retrying on a clock):

| The server says | From | Refusal | Decision |
|---|---|---|---|
| `The {name} is already in use by {viewer}!` | `Container.cs:740` | `Open InUse` | again in 3 s, doubling |
| `You do not yet have the right to loot the {name}.` | `Corpse.cs:133` | `Open NotYetOurs` | again in 5 s, doubling |
| `You may not loot the {name} because ...` | `Corpse.cs:129, :131` | `Open NeverOurs` | never |
| `Unable to put {item} into container` | `Player_Inventory.cs:1322` | `Put NoRoom` | never into that pack; once into another |
| `{name} is already a member of a Fellowship.` | `Entity/Fellowship.cs:110` | `Recruit AlreadyAMember` | again in 10 s, doubling |
| `{name} is busy.` | `Entity/Fellowship.cs:116, :128` | `Recruit Busy` | again in 10 s, same every time |
| `{name} is not accepting fellowship requests.` | `Player_Fellowship.cs:100` | `Recruit NotAccepting` | again in 10 s, doubling |
| `{name} declines your invite` | `Entity/Fellowship.cs:146` | `Recruit Declined` | again in 10 s, doubling |
| `You cannot attack {name}` | `Player_Melee.cs:122`, `Player_Missile.cs:110` | `Attack` | the target is given up for 90 s |
| `You must be a {mastery} to use the {essence}` | `PetDevice.cs:115` | `Summon NotForUs` | never |
| `{pet} is already active` | `PetDevice.cs:130`, `Pet.cs:133` | `Summon OneIsOut` | the essence is left ready |

**Read, decided, and waited on by nothing yet** -- they reach the log as
`the server refused (...)`, and the row is there to wire to the moment
something starts retrying on them:

| The server says | From | Refusal |
|---|---|---|
| `You cannot put {item} in that.` | `Player_Inventory.cs:855` | `Put NotThere` |
| `You are too encumbered to carry that!` | `Player_Inventory.cs:841, :1555, :2285, :2637, :2881` | `Carry` (the take-refusal event already steps the item over) |
| `You must first pick up the {item}` | `Player_Inventory.cs:1154` | `PickUpFirst` |
| `{spell} cannot be cast on {target}.` | `Player_Magic.cs:417` | `Cast` |
| `You must wield the {item} to use it.` / `You must contain the {item} to use it.` | `Player_Use.cs:73` | `Use` |
| `Cannot use the {item} with the {target}` | `Player_Use.cs:142` | `UseWith` |
| `The {item} is unsellable.` / `The {item} has no value and cannot be sold.` | `Player_Commerce.cs:271, :278` | `Sell` (the vendor run judges a sale by the item still being in the pack) |
| `You cannot sell that! The {item} is currently being traded.` / `... must be empty.` | `Player_Commerce.cs:285, :292` | `Sell` |
| `You are too encumbered to sell that!` / `You do not have enough free pack space to sell that!` | `Player_Commerce.cs:185, :187` | `Sell` |
| `You are too encumbered to buy that!` / `... enough pack space ...` / `... enough container slots ...` | `Vendor.cs:496, :498, :500` | `Buy` |

**Seen in the sweep and left out of the table**, because nothing here
asks for them and they are not answers to a request autoplay makes:
"You are out of ammunition!" (`Player_Combat.cs:852`, `Player_Missile.cs:253`;
the quiver is counted, not heard); the give and trade lines
("{Name} tries to give you ...", "You give {Name} ...", "You have accepted
the offer", "The items are being traded", "Trade confirmation failed...",
`Player_Inventory.cs:3292-3532`, `Player_Trade.cs:204-422`); the split and
merge complaints ("Split amount not valid!", "Stack not valid!", "You
cannot merge from vendor", "Stacks not compatible!", `Player_Inventory.cs:2250-3036`,
which are about a request malformed on this side); the fellowship
housekeeping lines ("... has given you permission to loot his or her
kills.", "You no longer have permission to loot anyone else's kills.",
"Your fellow {Name} has died!", "{Name} is now level {n}!", `Entity/Fellowship.cs:171-734`);
the allegiance answers (`Player_Allegiance.cs:92-1475`, of which "{patron}
is busy." shares its words with the recruit refusal and is told apart by
whom the character has invited); the Olthoi lines; "Cast efficiency:
{n}%" (`Player_Magic.cs:846`, a debugging aid); and the ACE-internal
"... failed!" strings ("TryDequipObjectWithNetworking failed!", "Item not
found!", "Target container not found!"), which are bugs on one side or
the other rather than refusals.

The rule for the next one: if the server refuses something in words and
the client retries it on a clock, add a row to `refusals::refused` with
the ACE file and line, a test in the exact wording, an arm in
`refusals::answer`, and a hand-off in `hear_refusal`. Not a
`strip_prefix` in the system that noticed.

## Getting there: height, wedges and clutter

A place is a point in three dimensions, and navigation that judges it on
the flat strands a character under its goal: a shopkeeper on an upper
floor was reached six millimetres away on the map and three metres
below on the ground floor. The rules that came of it:

- **A goal's height is believed** when the geometry says the point is
  in a known cell (`CollisionWorld::in_known_cell`). A cell id is not
  enough: the steering fills one in with a landblock, whose low word
  reads as "outdoors", and the goal was grounded onto the terrain.
- **Arrival counts height**, to within a doorsill, and so does reaching
  a waypoint: a waypoint at the top of a staircase is a pace away on the
  map.
- **One frame may not carry the character past what it is walking to.**
  Building a chunk of the navigation graph takes a couple of hundred
  milliseconds, and a walk charged all of it at running speed crossed
  the bottom step and back for ever.
- **A wedge lets go.** The rule that stops a character leaning on a wall
  clears the moment it is sent somewhere else, and otherwise rests a
  second and tries again: doors open and what was leaned on walks away.
- **What the server puts in the room is not in the ground.** The
  collision world and its graph are built from the DAT files and shared
  by every character in the process; a chest, a hook or a cart arrives
  and leaves with the packets. The walking physics goes through them
  and the server takes the position it is sent, so they never stall a
  walk -- a stall is always the landblock's own geometry -- but the
  retail client stopped at them. `ac_nav::obstacles` keeps the ones near
  the character as the cylinders the retail client collided them by
  (the Setup's `CylSphere`s, else its `Sphere`s), gathered again only
  when the world changes or the character moves. They are not laid over
  the ground the steering plans on; once the steering has chosen where
  to head, the leg there is walked round the first cylinder on it, a
  frame at a time, and a leg no detour clears is walked as it was. A
  creature, anything carried, anything Ethereal, a missile, anything
  collided by its parts' own BSP, and -- by the rule below -- a door are
  never obstacles.

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
- **A road is walked, not fought.** On its way somewhere -- a hunting
  ground, a counter, wherever a script or a hunting area's portal sends
  it -- a character fights only what attacks it, walks at it or a mate,
  or is fighting one of the party on the road, and lets a creature go
  once it stops following. The fighting is what the far end is for. A
  road is a road whoever planned it; only a roam or a patrol about the
  character's own ground is not one (`Client::on_its_way`,
  `Client::travel_about`).
