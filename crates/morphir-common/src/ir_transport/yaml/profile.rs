//! Native YAML vocabulary validation and strict document serialization.

use morphir_core::traversal::IrCursor;
use serde::{Serialize, de::DeserializeOwned};

use super::YamlCodec;
use crate::ir_transport::{IR_RECURSION_STACK_BYTES, SourceSpan, Stage, TransportDiagnostic};

pub(crate) fn validate_yaml_profile(input: &[u8]) -> Result<(), TransportDiagnostic> {
    let source = std::str::from_utf8(input).map_err(|error| {
        TransportDiagnostic::error(
            "morphir::ir::yaml::invalid_utf8",
            Stage::Syntax,
            IrCursor::root(),
            error.to_string(),
        )
        .with_guidance("encode the YAML artifact as UTF-8")
    })?;
    let mut saw_content = false;
    let mut block_scalar_parent_indent = None;
    let mut offset = 0;
    for (line_index, line) in source.split_inclusive('\n').enumerate() {
        let line_without_ending = line.trim_end_matches(['\r', '\n']);
        let trimmed = line_without_ending.trim_start();
        let indent = line_without_ending.len() - trimmed.len();
        let column = indent + 1;
        if trimmed.is_empty() {
            offset += line.len();
            continue;
        }
        if block_scalar_parent_indent.is_some_and(|parent| indent > parent) {
            offset += line.len();
            continue;
        }
        block_scalar_parent_indent = None;
        if trimmed.starts_with('#') {
            offset += line.len();
            continue;
        }
        if trimmed == "---" {
            if saw_content {
                return Err(profile_diagnostic(
                    "morphir::ir::yaml::multiple_documents",
                    Stage::Syntax,
                    "a YAML IR artifact contains more than one document",
                    "store exactly one IR document in each YAML artifact",
                    offset + column - 1,
                    3,
                    line_index + 1,
                    column,
                ));
            }
            offset += line.len();
            continue;
        }
        saw_content = true;
        if let Some((token_column, code, stage, message, guidance)) = forbidden_token(trimmed) {
            return Err(profile_diagnostic(
                code,
                stage,
                message,
                guidance,
                offset + column - 1 + token_column,
                1,
                line_index + 1,
                column + token_column,
            ));
        }
        if is_block_scalar_header(trimmed) {
            block_scalar_parent_indent = Some(indent);
        }
        offset += line.len();
    }
    Ok(())
}

pub(crate) fn decode_json_value(input: &[u8]) -> Result<serde_json::Value, TransportDiagnostic> {
    decode_document(input)
}

pub(crate) fn decode_document<T: DeserializeOwned>(input: &[u8]) -> Result<T, TransportDiagnostic> {
    validate_yaml_profile(input)?;
    stacker::grow(IR_RECURSION_STACK_BYTES, || {
        serde_saphyr::from_slice_with_options(input, YamlCodec::parse_options())
            .map_err(YamlCodec::decode_error)
    })
}

pub(crate) fn encode_document<T: Serialize>(value: &T) -> Result<Vec<u8>, TransportDiagnostic> {
    let mut rendered = stacker::grow(IR_RECURSION_STACK_BYTES, || {
        serde_saphyr::to_string_with_options(value, YamlCodec::serializer_options())
            .map_err(YamlCodec::encode_error)
    })?;
    rendered = rendered.replace("\r\n", "\n");
    if !rendered.ends_with('\n') {
        rendered.push('\n');
    }
    Ok(rendered.into_bytes())
}

fn is_block_scalar_header(line: &str) -> bool {
    let mut single_quoted = false;
    let mut double_quoted = false;
    let mut escaped = false;
    let mut comment_start = line.len();
    for (index, character) in line.char_indices() {
        if double_quoted {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                double_quoted = false;
            }
            continue;
        }
        if single_quoted {
            if character == '\'' {
                single_quoted = false;
            }
            continue;
        }
        match character {
            '\'' => single_quoted = true,
            '"' => double_quoted = true,
            '#' if token_boundary(line[..index].chars().next_back()) => {
                comment_start = index;
                break;
            }
            _ => {}
        }
    }
    let token = line[..comment_start]
        .split_whitespace()
        .next_back()
        .unwrap_or_default();
    let mut characters = token.chars();
    matches!(characters.next(), Some('|' | '>'))
        && characters.all(|character| matches!(character, '+' | '-' | '1'..='9'))
}

fn forbidden_token(line: &str) -> Option<(usize, &'static str, Stage, &'static str, &'static str)> {
    let mut single_quoted = false;
    let mut double_quoted = false;
    let mut escaped = false;
    let characters = line.char_indices().collect::<Vec<_>>();
    for (position, (index, character)) in characters.iter().copied().enumerate() {
        if double_quoted {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                double_quoted = false;
            }
            continue;
        }
        if single_quoted {
            if character == '\'' {
                if characters
                    .get(position + 1)
                    .is_some_and(|(_, next)| *next == '\'')
                {
                    continue;
                }
                single_quoted = false;
            }
            continue;
        }
        match character {
            '\'' => single_quoted = true,
            '"' => double_quoted = true,
            '#' if token_boundary(line[..index].chars().next_back()) => break,
            '&' | '*' if token_boundary(line[..index].chars().next_back()) => {
                return Some((
                    index,
                    "morphir::ir::yaml::alias_not_allowed",
                    Stage::Syntax,
                    "YAML anchors and aliases are disabled by the native IR profile",
                    "expand anchors and aliases into explicit YAML nodes",
                ));
            }
            '!' if token_boundary(line[..index].chars().next_back()) => {
                return Some((
                    index,
                    "morphir::ir::yaml::unsupported_tag",
                    Stage::Syntax,
                    "YAML semantic tags are not part of the native IR vocabulary",
                    "replace the tag with an explicit structural mapping",
                ));
            }
            '<' if line[index..].starts_with("<<:")
                && token_boundary(line[..index].chars().next_back()) =>
            {
                return Some((
                    index,
                    "morphir::ir::yaml::merge_key_not_allowed",
                    Stage::Syntax,
                    "YAML merge keys are disabled by the native IR profile",
                    "expand the merge as ordinary mapping entries",
                ));
            }
            _ => {}
        }
    }

    for (index, token) in unquoted_scalar_tokens(line) {
        let lowercase = token.to_ascii_lowercase();
        if matches!(lowercase.as_str(), ".inf" | "+.inf" | "-.inf" | ".nan") {
            return Some((
                index,
                "morphir::ir::yaml::non_finite_number",
                Stage::Normalization,
                "non-finite YAML numbers cannot represent concrete IR literals losslessly",
                "use a finite number representable by the concrete IR literal",
            ));
        }
        if looks_like_timestamp(token) {
            return Some((
                index,
                "morphir::ir::yaml::ambiguous_scalar",
                Stage::Normalization,
                "an unquoted timestamp-like scalar is ambiguous in the native IR profile",
                "quote the value to store it as a string",
            ));
        }
    }
    None
}

fn unquoted_scalar_tokens(line: &str) -> Vec<(usize, &str)> {
    let characters = line.char_indices().collect::<Vec<_>>();
    let mut tokens = Vec::new();
    let mut token_start = None;
    let mut single_quoted = false;
    let mut double_quoted = false;
    let mut escaped = false;
    let mut skip_escaped_single_quote = false;

    for (position, (index, character)) in characters.iter().copied().enumerate() {
        if double_quoted {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                double_quoted = false;
            }
            continue;
        }
        if single_quoted {
            if skip_escaped_single_quote {
                skip_escaped_single_quote = false;
            } else if character == '\'' {
                if characters
                    .get(position + 1)
                    .is_some_and(|(_, next)| *next == '\'')
                {
                    skip_escaped_single_quote = true;
                } else {
                    single_quoted = false;
                }
            }
            continue;
        }

        if matches!(character, '\'' | '"') {
            push_unquoted_token(&mut tokens, line, &mut token_start, index);
            single_quoted = character == '\'';
            double_quoted = character == '"';
        } else if character == '#' && token_boundary(line[..index].chars().next_back()) {
            push_unquoted_token(&mut tokens, line, &mut token_start, index);
            break;
        } else if character.is_whitespace()
            || matches!(character, ':' | ',' | '[' | ']' | '{' | '}')
        {
            push_unquoted_token(&mut tokens, line, &mut token_start, index);
        } else {
            token_start.get_or_insert(index);
        }
    }
    push_unquoted_token(&mut tokens, line, &mut token_start, line.len());
    tokens
}

fn push_unquoted_token<'line>(
    tokens: &mut Vec<(usize, &'line str)>,
    line: &'line str,
    token_start: &mut Option<usize>,
    end: usize,
) {
    if let Some(start) = token_start.take() {
        tokens.push((start, &line[start..end]));
    }
}

fn token_boundary(character: Option<char>) -> bool {
    character.is_none_or(|character| {
        character.is_whitespace() || matches!(character, ':' | ',' | '[' | ']' | '{' | '}' | '-')
    })
}

fn looks_like_timestamp(token: &str) -> bool {
    let bytes = token.as_bytes();
    bytes.len() == 10
        && bytes[0..4].iter().all(u8::is_ascii_digit)
        && bytes[4] == b'-'
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[7] == b'-'
        && bytes[8..10].iter().all(u8::is_ascii_digit)
}

#[allow(clippy::too_many_arguments)]
fn profile_diagnostic(
    code: &'static str,
    stage: Stage,
    message: &'static str,
    guidance: &'static str,
    offset: usize,
    length: usize,
    line: usize,
    column: usize,
) -> TransportDiagnostic {
    TransportDiagnostic::error(code, stage, IrCursor::root(), message)
        .with_guidance(guidance)
        .with_source_span(SourceSpan {
            offset,
            length,
            line,
            column,
        })
}
