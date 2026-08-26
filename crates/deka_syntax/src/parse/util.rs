//! Parser helpers: operator precedence, token names, and EOF sentinel.

use crate::ast::{BinOp, Span};
use crate::lexer::{Token, TokenKind};

pub(super) fn infix_info(kind: TokenKind) -> Option<(u8, u8, BinOp)> {
    use TokenKind::*;
    Some(match kind {
        Or => (1, 2, BinOp::Or),
        And => (3, 4, BinOp::And),
        EqEq => (5, 6, BinOp::Eq),
        NotEq => (5, 6, BinOp::Ne),
        Lt => (7, 8, BinOp::Lt),
        Le => (7, 8, BinOp::Le),
        Gt => (7, 8, BinOp::Gt),
        Ge => (7, 8, BinOp::Ge),
        Plus => (9, 10, BinOp::Add),
        Minus => (9, 10, BinOp::Sub),
        Star => (11, 12, BinOp::Mul),
        Slash => (11, 12, BinOp::Div),
        Percent => (11, 12, BinOp::Mod),
        _ => return std::option::Option::None,
    })
}

pub(super) fn eof_token<'a>() -> Token<'a> {
    Token {
        kind: TokenKind::Eof,
        text: "",
        span: Span::dummy(),
    }
}

pub(super) fn token_name(kind: TokenKind) -> &'static str {
    use TokenKind::*;
    match kind {
        Number => "number",
        BigInt => "bigint",
        String => "string",
        True => "`true`",
        False => "`false`",
        None => "`none`",
        Identifier => "identifier",
        Const => "`const`",
        Let => "`let`",
        Mut => "`mut`",
        Function => "`function`",
        Fn => "`fn`",
        Struct => "`struct`",
        Enum => "`enum`",
        Type => "`type`",
        Import => "`import`",
        Export => "`export`",
        From => "`from`",
        As => "`as`",
        If => "`if`",
        Else => "`else`",
        For => "`for`",
        Return => "`return`",
        Match => "`match`",
        Unsafe => "`unsafe`",
        Await => "`await`",
        Async => "`async`",
        Pub => "`pub`",
        Plus => "`+`",
        Minus => "`-`",
        Star => "`*`",
        Slash => "`/`",
        Percent => "`%`",
        Eq => "`=`",
        EqEq => "`==`",
        NotEq => "`!=`",
        Lt => "`<`",
        Le => "`<=`",
        Gt => "`>`",
        Ge => "`>=`",
        And => "`&&`",
        Or => "`||`",
        Not => "`!`",
        Ampersand => "`&`",
        Pipe => "`|`",
        Caret => "`^`",
        Shl => "`<<`",
        Shr => "`>>`",
        LParen => "`(`",
        RParen => "`)`",
        LBrace => "`{`",
        RBrace => "`}`",
        LBracket => "`[`",
        RBracket => "`]`",
        Comma => "`,`",
        Semicolon => "`;`",
        Colon => "`:`",
        DoubleColon => "`::`",
        Dot => "`.`",
        Arrow => "`->`",
        FatArrow => "`=>`",
        Question => "`?`",
        Spread => "`...`",
        LtJsx => "`<`",
        GtJsx => "`>`",
        SlashJsx => "`/`",
        Newline => "newline",
        Comment => "comment",
        Eof => "end of file",
        Error => "error",
    }
}
