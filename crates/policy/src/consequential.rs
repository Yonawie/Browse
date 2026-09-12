//! Deterministic classification of irreversible ("consequential") actions
//! (ADR-005 §6). The model never decides this.

use core_types::{ConsequentialPredicate, ToolCall, ToolManifest};

/// Multilingual lexicon of verbs/labels that indicate an irreversible action.
/// Matching is case-insensitive on the accessible name of the target.
pub const CONSEQUENTIAL_LEXICON: &[&str] = &[
    // en
    "pay", "buy", "purchase", "place order", "checkout", "check out", "confirm order", "send", "submit", "delete",
    "remove", "publish", "post", "transfer", "book", "reserve", "subscribe", "unsubscribe", "cancel subscription",
    "accept offer", "sign", "approve", "merge", "deploy", "upload",
    // ru
    "оплатить", "купить", "оформить заказ", "заказать", "подтвердить заказ", "отправить", "удалить", "опубликовать",
    "перевести", "забронировать", "подписаться", "отписаться", "подписать", "одобрить", "загрузить",
    // de
    "kaufen", "bezahlen", "bestellen", "senden", "löschen", "veröffentlichen", "überweisen", "buchen",
    // fr
    "acheter", "payer", "commander", "envoyer", "supprimer", "publier", "virer", "réserver",
    // es
    "comprar", "pagar", "pedir", "enviar", "eliminar", "borrar", "publicar", "transferir", "reservar",
    // he / ar / zh / ja (common forms)
    "שלח", "מחק", "קנה", "شراء", "إرسال", "حذف", "购买", "付款", "发送", "删除", "購入", "送信", "削除",
];

/// Words that indicate the action is *not* consequential even if a lexicon
/// word appears as a substring (e.g. "Send feedback" link vs. "Send" a message
/// is ambiguous; we keep it consequential — safety first — but exclude clear
/// navigational labels).
const NEGATIVE_LEXICON: &[&str] = &["learn more", "подробнее", "how to", "как ", "faq"];

const CHECKOUT_URL_MARKERS: &[&str] = &["/checkout", "/payment", "/pay", "/order/confirm", "/purchase", "/billing"];

pub fn name_matches_lexicon(name: &str) -> bool {
    let lower = name.trim().to_lowercase();
    if lower.is_empty() {
        return false;
    }
    if NEGATIVE_LEXICON.iter().any(|n| lower.contains(n)) {
        return false;
    }
    CONSEQUENTIAL_LEXICON.iter().any(|w| {
        // whole-word-ish match: the label starts with the verb or contains it delimited by spaces
        lower == *w || lower.starts_with(&format!("{w} ")) || lower.contains(&format!(" {w} ")) || lower.ends_with(&format!(" {w}"))
    })
}

pub fn looks_like_payment(call: &ToolCall) -> bool {
    if let Some(t) = &call.target {
        if t.form_has_payment_fields {
            return true;
        }
        let n = t.name.to_lowercase();
        if ["pay", "оплат", "checkout", "bezahlen", "payer", "pagar", "付款", "purchase", "buy now", "купить"].iter().any(|w| n.contains(w)) {
            return true;
        }
        if let Some(h) = &t.href {
            if CHECKOUT_URL_MARKERS.iter().any(|m| h.to_lowercase().contains(m)) {
                return true;
            }
        }
    }
    if let Some(url) = call.args.get("url").and_then(|v| v.value.as_str()) {
        if CHECKOUT_URL_MARKERS.iter().any(|m| url.to_lowercase().contains(m)) {
            return true;
        }
    }
    false
}

/// A call is consequential if its manifest says so unconditionally, if the
/// site's WebMCP tool carries `consequentialHint`, or if any of the manifest's
/// `consequential_when` predicates holds for the target.
pub fn is_consequential(call: &ToolCall, manifest: &ToolManifest) -> bool {
    if manifest.safety.consequential {
        return true;
    }
    let Some(target) = &call.target else {
        return manifest
            .safety
            .consequential_when
            .contains(&ConsequentialPredicate::NavigatesToCheckoutLikeUrl)
            && looks_like_payment(call);
    };
    if target.site_tool_consequential_hint {
        return true;
    }
    manifest.safety.consequential_when.iter().any(|p| match p {
        ConsequentialPredicate::TargetIsSubmit => target.is_submit,
        ConsequentialPredicate::TargetTextMatchesConsequentialLexicon => name_matches_lexicon(&target.name),
        ConsequentialPredicate::FormHasPaymentFields => target.form_has_payment_fields,
        ConsequentialPredicate::FormHasFileUpload => target.form_has_file_upload,
        ConsequentialPredicate::NavigatesToCheckoutLikeUrl => looks_like_payment(call),
        ConsequentialPredicate::DeletesOrPublishes => {
            let n = target.name.to_lowercase();
            ["delete", "remove", "publish", "удал", "опублик", "löschen", "supprimer", "eliminar", "删除", "削除"].iter().any(|w| n.contains(w))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexicon_matches_multilingual_labels() {
        for label in ["Pay now", "Оплатить", "Place order", "Опубликовать запись", "Bestellen", "Envoyer", "购买", "Delete", "Sign"] {
            assert!(name_matches_lexicon(label), "{label}");
        }
        for label in ["Next", "Далее", "Learn more about sending", "Как оплатить", "FAQ", "Search"] {
            assert!(!name_matches_lexicon(label), "{label}");
        }
    }
}
