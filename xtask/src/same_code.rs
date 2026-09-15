//! `same-code`: proves a change touched comments only.
//!
//! Each changed `.rs` file is parsed at the base revision and in the working tree, doc
//! attributes (`///`, `//!`, `/** */`, `#[doc = ..]`) are stripped at every depth, macro
//! bodies included, and the token streams compared. Plain comments and layout are not
//! tokens. `#[doc(hidden)]` and other `doc(..)` lists are code.

use std::process::ExitCode;

use anyhow::Result;
use proc_macro2::{Delimiter, Group, TokenStream, TokenTree};
use quote::ToTokens;
use syn::{ImplItem, Item, TraitItem};

use crate::git::Repo;

pub fn run(base: &str, allow_new: bool) -> Result<ExitCode> {
    let repo = Repo::here()?;
    repo.check_rev(base)?;
    let files = repo.changed_rs_files(base)?;
    let mut differing = 0;
    for path in &files {
        let old = repo.show(base, path)?;
        let new = repo.read(path)?;
        let outcome = compare_sources(old.as_deref(), new.as_deref());
        if let Some(message) = describe(path, base, &outcome, allow_new) {
            println!("{message}");
            differing += 1;
        }
    }
    let changed = files.len();
    if differing == 0 {
        println!("same code in {changed} changed .rs file(s) since {base}");
        Ok(ExitCode::SUCCESS)
    } else {
        println!("code differs in {differing} of {changed} changed .rs file(s) since {base}");
        Ok(ExitCode::FAILURE)
    }
}

/// What comparing one file's two versions found.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Same,
    Differs(Difference),
    /// Only the working tree has the file.
    Added,
    /// Only the base revision has the file.
    Deleted,
    Unparsable {
        side: Side,
        line: usize,
        error: String,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub enum Side {
    Base,
    WorkingTree,
}

/// The first place two versions of a file differ in code.
#[derive(Debug, PartialEq, Eq)]
pub struct Difference {
    /// The item, nested as `impl Client > fn tick`.
    pub place: String,
    pub change: Change,
    /// 1-based line of the item's first code token: in the base revision for
    /// [`Change::Removed`], in the working tree otherwise.
    pub line: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Change {
    Changed,
    Added,
    Removed,
}

/// Compares a file's base and working-tree text; `None` is a side without the file.
pub fn compare_sources(old: Option<&str>, new: Option<&str>) -> Outcome {
    let (old, new) = match (old, new) {
        (Some(old), Some(new)) => (old, new),
        (None, Some(_)) => return Outcome::Added,
        (Some(_), None) => return Outcome::Deleted,
        (None, None) => return Outcome::Same,
    };
    let parse = |text: &str, side: Side| {
        syn::parse_file(text).map_err(|e| Outcome::Unparsable {
            side,
            line: e.span().start().line,
            error: e.to_string(),
        })
    };
    let old = match parse(old, Side::Base) {
        Ok(file) => file,
        Err(outcome) => return outcome,
    };
    let new = match parse(new, Side::WorkingTree) {
        Ok(file) => file,
        Err(outcome) => return outcome,
    };
    match first_difference(&old, &new) {
        Some(difference) => Outcome::Differs(difference),
        None => Outcome::Same,
    }
}

/// One report line for a file whose outcome counts as a difference, else `None`.
pub fn describe(path: &str, base: &str, outcome: &Outcome, allow_new: bool) -> Option<String> {
    let line = match outcome {
        Outcome::Same => return None,
        Outcome::Added | Outcome::Deleted if allow_new => return None,
        Outcome::Added => format!("{path}: new file (--allow-new allows it)"),
        Outcome::Deleted => format!("{path}: deleted (--allow-new allows it)"),
        Outcome::Unparsable { side, line, error } => {
            let side = match side {
                Side::Base => base,
                Side::WorkingTree => "the working tree",
            };
            format!("{path}:{line}: does not parse in {side}: {error}")
        }
        Outcome::Differs(Difference {
            place,
            change,
            line,
        }) => match change {
            Change::Changed => format!("{path}:{line}: code differs, first in {place}"),
            Change::Added => format!("{path}:{line}: code differs, first at added {place}"),
            Change::Removed => {
                format!("{path}: code differs, first at removed {place} (line {line} in {base})")
            }
        },
    };
    Some(line)
}

/// Where `old` and `new` first differ once doc attributes are stripped, or `None`
/// when their code is the same.
pub fn first_difference(old: &syn::File, new: &syn::File) -> Option<Difference> {
    if code(old) == code(new) {
        return None;
    }
    let found = if code_of_all(&old.attrs) != code_of_all(&new.attrs) {
        None
    } else {
        compare_lists(&old.items, &new.items, "")
    };
    Some(found.unwrap_or_else(|| Difference {
        place: "the file's inner attributes".to_owned(),
        change: Change::Changed,
        line: new.attrs.first().map_or(1, first_line),
    }))
}

/// Removes every `#[doc = ..]` and `#![doc = ..]`, which is what `///`, `//!` and
/// `/** */` become, at every depth of `tokens`.
pub fn strip_docs(tokens: TokenStream) -> TokenStream {
    let tokens: Vec<TokenTree> = tokens.into_iter().collect();
    let mut kept = Vec::with_capacity(tokens.len());
    let mut at = 0;
    while at < tokens.len() {
        if let Some(after) = doc_attribute_end(&tokens, at) {
            at = after;
            continue;
        }
        kept.push(match &tokens[at] {
            TokenTree::Group(group) => {
                let mut stripped = Group::new(group.delimiter(), strip_docs(group.stream()));
                stripped.set_span(group.span());
                TokenTree::Group(stripped)
            }
            other => other.clone(),
        });
        at += 1;
    }
    kept.into_iter().collect()
}

/// When a doc attribute starts at `at`, the index just past it.
fn doc_attribute_end(tokens: &[TokenTree], at: usize) -> Option<usize> {
    let punct_at =
        |i: usize, c: char| matches!(tokens.get(i), Some(TokenTree::Punct(p)) if p.as_char() == c);
    if !punct_at(at, '#') {
        return None;
    }
    let body = if punct_at(at + 1, '!') {
        at + 2
    } else {
        at + 1
    };
    let TokenTree::Group(group) = tokens.get(body)? else {
        return None;
    };
    if group.delimiter() != Delimiter::Bracket {
        return None;
    }
    let mut inside = group.stream().into_iter();
    let named_doc = matches!(inside.next(), Some(TokenTree::Ident(name)) if name == "doc");
    let assigned = matches!(inside.next(), Some(TokenTree::Punct(p)) if p.as_char() == '=');
    (named_doc && assigned).then_some(body + 1)
}

fn code<T: ToTokens>(node: &T) -> String {
    strip_docs(node.to_token_stream()).to_string()
}

fn code_of_all<T: ToTokens>(nodes: &[T]) -> String {
    let mut tokens = TokenStream::new();
    for node in nodes {
        node.to_tokens(&mut tokens);
    }
    strip_docs(tokens).to_string()
}

fn first_line<T: ToTokens>(node: &T) -> usize {
    strip_docs(node.to_token_stream())
        .into_iter()
        .next()
        .map_or(1, |token| token.span().start().line)
}

/// An item that can hold items: impls, traits and inline modules.
trait Node: ToTokens {
    fn label(&self) -> String;

    /// With `self` and `other` differing: the first difference among their items
    /// when everything else about them is the same.
    fn inner_difference(&self, _other: &Self, _place: &str) -> Option<Difference> {
        None
    }
}

fn compare_lists<T: Node>(olds: &[T], news: &[T], within: &str) -> Option<Difference> {
    for index in 0..olds.len().max(news.len()) {
        let difference = match (olds.get(index), news.get(index)) {
            (Some(old), Some(new)) if code(old) == code(new) => continue,
            (Some(old), Some(new)) => {
                let (old_label, new_label) = (old.label(), new.label());
                let place = nested(within, &new_label);
                old.inner_difference(new, &place)
                    .unwrap_or_else(|| Difference {
                        place: if old_label == new_label {
                            place
                        } else {
                            nested(within, &format!("{new_label} (was {old_label})"))
                        },
                        change: Change::Changed,
                        line: first_line(new),
                    })
            }
            (Some(old), None) => Difference {
                place: nested(within, &old.label()),
                change: Change::Removed,
                line: first_line(old),
            },
            (None, Some(new)) => Difference {
                place: nested(within, &new.label()),
                change: Change::Added,
                line: first_line(new),
            },
            (None, None) => unreachable!("the index is below the longer list's length"),
        };
        return Some(difference);
    }
    None
}

fn nested(within: &str, label: &str) -> String {
    if within.is_empty() {
        label.to_owned()
    } else {
        format!("{within} > {label}")
    }
}

/// `node`'s code with the items inside it cleared, for comparing everything else.
fn shell<T: Clone + ToTokens>(node: &T, clear: impl FnOnce(&mut T)) -> String {
    let mut shell = node.clone();
    clear(&mut shell);
    code(&shell)
}

impl Node for Item {
    fn label(&self) -> String {
        match self {
            Item::Const(item) => format!("const {}", item.ident),
            Item::Enum(item) => format!("enum {}", item.ident),
            Item::ExternCrate(item) => format!("extern crate {}", item.ident),
            Item::Fn(item) => format!("fn {}", item.sig.ident),
            Item::ForeignMod(_) => "extern block".to_owned(),
            Item::Impl(item) => match &item.trait_ {
                Some((negated, path, _)) => format!(
                    "impl {}{} for {}",
                    if negated.is_some() { "!" } else { "" },
                    compact(path),
                    compact(&item.self_ty)
                ),
                None => format!("impl {}", compact(&item.self_ty)),
            },
            Item::Macro(item) => match &item.ident {
                Some(name) => format!("macro_rules! {name}"),
                None => format!("{}!", compact(&item.mac.path)),
            },
            Item::Mod(item) => format!("mod {}", item.ident),
            Item::Static(item) => format!("static {}", item.ident),
            Item::Struct(item) => format!("struct {}", item.ident),
            Item::Trait(item) => format!("trait {}", item.ident),
            Item::TraitAlias(item) => format!("trait {}", item.ident),
            Item::Type(item) => format!("type {}", item.ident),
            Item::Union(item) => format!("union {}", item.ident),
            Item::Use(item) => format!("use {}", compact(&item.tree)),
            _ => "item".to_owned(),
        }
    }

    fn inner_difference(&self, other: &Self, place: &str) -> Option<Difference> {
        match (self, other) {
            (Item::Impl(old), Item::Impl(new)) => {
                let clear = |item: &mut syn::ItemImpl| item.items.clear();
                (shell(old, clear) == shell(new, clear))
                    .then(|| compare_lists(&old.items, &new.items, place))?
            }
            (Item::Trait(old), Item::Trait(new)) => {
                let clear = |item: &mut syn::ItemTrait| item.items.clear();
                (shell(old, clear) == shell(new, clear))
                    .then(|| compare_lists(&old.items, &new.items, place))?
            }
            (Item::Mod(old), Item::Mod(new)) => {
                let clear = |item: &mut syn::ItemMod| {
                    if let Some((_, items)) = &mut item.content {
                        items.clear();
                    }
                };
                let (Some((_, old_items)), Some((_, new_items))) = (&old.content, &new.content)
                else {
                    return None;
                };
                (shell(old, clear) == shell(new, clear))
                    .then(|| compare_lists(old_items, new_items, place))?
            }
            _ => None,
        }
    }
}

impl Node for ImplItem {
    fn label(&self) -> String {
        match self {
            ImplItem::Const(item) => format!("const {}", item.ident),
            ImplItem::Fn(item) => format!("fn {}", item.sig.ident),
            ImplItem::Type(item) => format!("type {}", item.ident),
            ImplItem::Macro(item) => format!("{}!", compact(&item.mac.path)),
            _ => "item".to_owned(),
        }
    }
}

impl Node for TraitItem {
    fn label(&self) -> String {
        match self {
            TraitItem::Const(item) => format!("const {}", item.ident),
            TraitItem::Fn(item) => format!("fn {}", item.sig.ident),
            TraitItem::Type(item) => format!("type {}", item.ident),
            TraitItem::Macro(item) => format!("{}!", compact(&item.mac.path)),
            _ => "item".to_owned(),
        }
    }
}

/// Tokens printed close to how they are written: `Vec<u8>`, not `Vec < u8 >`.
fn compact<T: ToTokens>(node: &T) -> String {
    node.to_token_stream()
        .to_string()
        .replace(" :: ", "::")
        .replace(":: ", "::")
        .replace(" < ", "<")
        .replace("< ", "<")
        .replace(" >", ">")
        .replace(" ,", ",")
        .replace("& ", "&")
        .replace("{ ", "{")
        .replace(" }", "}")
}

#[cfg(test)]
mod tests;
