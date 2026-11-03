# Quests: plan

Agreed 2026-11-02. A quest is completable only if its items are looted, so the quest list and the
loot profile are one idea: the profile keeps what an active quest wants, and autoplay does the quests
the player switched on. Phase 1 (Warden portal flags, `quest_flags.rs`) is built.

## The data ships with acswarm

`crates/ac-world/data/quests.csv`, generated from ACE's world database by a `tools/` script, as
portals, unlocks, hunting grounds and creatures already are. Players need no ACE copy. Per quest:
- the giver: name, place with its cell;
- what it asks: kill N of a creature, bring items (wcid and count), or talk to someone;
- where its items come from: the creatures that drop them and the chance;
- the reward: XP, items, a flag or area it unlocks; the wait before it repeats.

## Quests switched on and off (autoplay settings)

- A Quests section: "do quests" on/off, and each quest with an on/off box.
- Each quest shows a state: ready to hand in; needs 3 more X; won't complete (a loot rule above the
  quest rule sells, salvages or skips its item, named); can't do yet (drop or kill too high level,
  the giver's area locked).
- Autoplay takes on only quests that are on and not blocked. A blocked one says which rule to move.

## The loot profile keeps what an active quest wants

- A new rule condition, "wanted by an active quest": the item is accepted by a giver of a quest that
  is on, and fewer are carried than the quest needs.
- The default profile has one rule near the top, group "Quests": Keep, wanted by an active quest.
  It is an ordinary rule: moved, turned off, or overruled by rules above it. The profile decides.
- A quest switched off lets its items go to the rules below.

## Hunting counts a quest's reward

The ground picker works in XP per hour. An active quest adds its reward spread over the expected
time to its kills or drops, so a ground where its items drop can win over a slightly better one.

## The errand run: many stops, anywhere

The town run becomes a list of stops: sell, restock, and each giver with a ready hand-in. Every
stop is a `Place` with its cell, so a giver in a dungeon is as reachable as a counter.
- The journey planner prices each leg (portals, recalls, `/lifestone`, the town network); with at
  most about six stops every order is tried and the fastest taken.
- A turn-in joins a run when its giver is in a town the run visits anyway or a short leg from one,
  or when its XP alone beats the detour time at the character's XP per hour. Otherwise it waits:
  the items are kept, and the next run that passes takes it.
- A repeat wait comes from the data; the server's "you must wait" refusal is a row in
  `ac_agent::refusals` and the giver is not asked again until it is over.
- A giver no route reaches is refused when the run is planned, not found out on arrival.

## Steps

1. `quests.csv` bundled; the quest list and states shown (no behaviour change). Built: data on
   `quests`, the states, the Quests section and the kill count on `quests-list`.
2. The rule condition, the default rule, the profile cross-check. Built (branch `quests-loot`):
   `Ask::Quest`, the starter's "quest items" rule first, the wants re-judging the pack, the
   cross-check naming the blocking rule, a mate's wants calling a body's item to it.
3. Kill and talk quests: the reward in the ground picker; the giver as a stop.
4. Item turn-ins and the errand run with many stops.

Later: items picked up off the ground rather than from corpses. Starts after movement phase 4
merges: the errand run is built on phase 4's journeys, whose steps keep their cells.
