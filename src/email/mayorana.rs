// src/email/mayorana.rs
//
// The emails a mayorana.ch subscriber gets about their plan: the licence, the
// cancellation and the one "come back" reminder. They go out in the language
// of the site the plan was bought from (subscriptions.locale), from Mohamed by
// name, and a reply reaches him (MAYORANA_REPLY_TO).
//
// The wording is the one agreed in the "Subscriber email copy" doc; change it
// there first, then here.

use super::{escape_html, plans_page, POSTAL_ADDRESS};
use chrono::{DateTime, Datelike, Utc};

/// The two languages mayorana.ch is written in. Anything else is English.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Fr,
}

impl Lang {
    /// From a locale tag as the site sends it ("fr", "fr-CH", "en"…).
    pub fn from_tag(tag: Option<&str>) -> Self {
        match tag.map(|t| t.trim().to_ascii_lowercase()) {
            Some(t) if t == "fr" || t.starts_with("fr-") || t.starts_with("fr_") => Self::Fr,
            _ => Self::En,
        }
    }

    pub fn tag(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Fr => "fr",
        }
    }

    /// The From name of the plan emails.
    pub fn sender_name(self) -> &'static str {
        match self {
            Self::En => "Mohamed from Mayorana",
            Self::Fr => "Mohamed de Mayorana",
        }
    }
}

const MONTHS_EN: [&str; 12] = [
    "January", "February", "March", "April", "May", "June", "July", "August", "September", "October",
    "November", "December",
];
const MONTHS_FR: [&str; 12] = [
    "janvier", "février", "mars", "avril", "mai", "juin", "juillet", "août", "septembre", "octobre",
    "novembre", "décembre",
];

/// A day as a reader of that language writes it: "1 November 2026", "1er novembre 2026".
pub fn format_date(date: DateTime<Utc>, lang: Lang) -> String {
    let month = date.month0() as usize;
    match lang {
        Lang::En => format!("{} {} {}", date.day(), MONTHS_EN[month], date.year()),
        Lang::Fr if date.day() == 1 => format!("1er {} {}", MONTHS_FR[month], date.year()),
        Lang::Fr => format!("{} {} {}", date.day(), MONTHS_FR[month], date.year()),
    }
}

/// Where the apps are listed, in the reader's language.
fn apps_page(lang: Lang) -> String {
    format!("https://mayorana.ch/{}/apps", lang.tag())
}

fn link(url: &str, text: &str) -> String {
    format!(r#"<a href="{url}" style="color:#0F172A">{text}</a>"#)
}

fn button(url: &str, text: &str) -> String {
    format!(
        r#"<p><a href="{url}" style="display:inline-block;padding:10px 20px;background:#0F172A;color:white;text-decoration:none;border-radius:6px">{text}</a></p>"#
    )
}

const MUTED: &str = r#"style="color:#64748B;font-size:13px""#;

// ── The licence ──────────────────────────────────────────────────────────────

pub fn licence_subject(plan_name: &str, lang: Lang) -> String {
    match lang {
        Lang::En => format!("Your {plan_name} licence"),
        Lang::Fr => format!("Votre licence {plan_name}"),
    }
}

/// `renews_on` is set for a paid plan, as `format_date` writes it; the free
/// allowance has none and gets the shorter email.
pub fn licence_body(
    plan_name: &str,
    licence: &str,
    apps: &str,
    monthly_credits: i64,
    renews_on: Option<&str>,
    lang: Lang,
) -> String {
    let code = format!(
        r#"<pre style="font-size:20px;letter-spacing:2px;background:#F1F5F9;padding:12px;border-radius:6px;text-align:center">{licence}</pre>"#
    );
    let page = plans_page();
    let content = match (renews_on, lang) {
        (Some(renews_on), Lang::En) => {
            let scope = match apps {
                "*" => "The same licence works in every app in your plan.".to_string(),
                "" => "It gives the apps' AI features their monthly credits.".to_string(),
                list => format!("It turns on Pro in {list}."),
            };
            format!(
                r#"<p>Hi,</p>
<p>Thanks for subscribing to {plan_name}. Here is your licence:</p>
{code}
<p>To turn it on, open any Mayorana app, click <strong>Get Pro…</strong> and paste it. {scope}</p>
<p>Don't have an app yet? Each one installs with one line from its page on {apps_link}.</p>
<p>You have <strong>{monthly_credits} AI credits</strong> this month, and they refill on {renews_on}. Your plan, invoices and card are on {sub_link}, where you can also cancel at any time.</p>
<p>If anything doesn't work, just reply. This comes straight to me.</p>
<p>Mohamed<br><span style="color:#64748B">Founder, Mayorana</span></p>
<p {MUTED}>Keep this email: the same licence works on every computer you use. To see your plan, sign in with this email address at {page}.</p>"#,
                apps_link = link(&apps_page(lang), "mayorana.ch/apps"),
                sub_link = link(&page, "your subscription page"),
            )
        }
        (Some(renews_on), Lang::Fr) => {
            let scope = match apps {
                "*" => "La même licence fonctionne dans toutes les apps de votre formule.".to_string(),
                "" => "Elle donne aux fonctions IA des apps leurs crédits du mois.".to_string(),
                list => format!("Elle active Pro dans {list}."),
            };
            format!(
                r#"<p>Bonjour,</p>
<p>Merci pour votre abonnement à {plan_name}. Voici votre licence :</p>
{code}
<p>Pour l'activer, ouvrez n'importe quelle app Mayorana, cliquez sur <strong>Get Pro…</strong> et collez-la. {scope}</p>
<p>Pas encore d'app ? Chacune s'installe en une ligne depuis sa page sur {apps_link}.</p>
<p>Vous avez <strong>{monthly_credits} crédits IA</strong> ce mois-ci, renouvelés le {renews_on}. Votre formule, vos factures et votre carte sont sur {sub_link}, où vous pouvez aussi résilier à tout moment.</p>
<p>Si quelque chose ne fonctionne pas, répondez simplement à cet e-mail. Il arrive directement chez moi.</p>
<p>Mohamed<br><span style="color:#64748B">Fondateur, Mayorana</span></p>
<p {MUTED}>Gardez cet e-mail : la même licence fonctionne sur tous vos ordinateurs. Pour voir votre formule, connectez-vous avec cette adresse e-mail sur {page}.</p>"#,
                apps_link = link(&apps_page(lang), "mayorana.ch/apps"),
                sub_link = link(&page, "votre page d'abonnement"),
            )
        }
        (None, Lang::En) => format!(
            r#"<p>Hi,</p>
<p>Here is your free licence. Paste it into the app's licence window to use the apps' AI features.</p>
{code}
<p>It includes <strong>{monthly_credits} AI credits</strong> every month, and they refill on their own.</p>
<p>If anything doesn't work, just reply. This comes straight to me.</p>
<p>Mohamed<br><span style="color:#64748B">Founder, Mayorana</span></p>
<p {MUTED}>Keep this email: the same licence works on every computer you use.</p>"#
        ),
        (None, Lang::Fr) => format!(
            r#"<p>Bonjour,</p>
<p>Voici votre licence gratuite. Collez-la dans la fenêtre de licence de l'app pour utiliser ses fonctions IA.</p>
{code}
<p>Elle comprend <strong>{monthly_credits} crédits IA</strong> chaque mois, renouvelés automatiquement.</p>
<p>Si quelque chose ne fonctionne pas, répondez simplement à cet e-mail. Il arrive directement chez moi.</p>
<p>Mohamed<br><span style="color:#64748B">Fondateur, Mayorana</span></p>
<p {MUTED}>Gardez cet e-mail : la même licence fonctionne sur tous vos ordinateurs.</p>"#
        ),
    };
    layout(&content, None, lang)
}

// ── The cancellation ─────────────────────────────────────────────────────────

pub fn cancelled_subject(plan_name: &str, ended: bool, lang: Lang) -> String {
    match (ended, lang) {
        (false, Lang::En) => format!("Your {plan_name} plan is cancelled. Can I ask why?"),
        (false, Lang::Fr) => format!("Votre formule {plan_name} est résiliée. Puis-je vous demander pourquoi ?"),
        (true, Lang::En) => format!("Your {plan_name} plan has ended"),
        (true, Lang::Fr) => format!("Votre formule {plan_name} a pris fin"),
    }
}

/// `ended`: the plan is already over (Stripe gave up on a card, or it was
/// stopped at once), so there is nothing left to resume and the reader may not
/// have chosen this. Otherwise it runs until `ends_on`.
pub fn cancelled_body(plan_name: &str, ends_on: &str, credits_left: i64, ended: bool, lang: Lang) -> String {
    let page = plans_page();
    let content = match (ended, lang) {
        (false, Lang::En) => format!(
            r#"<p>Hi,</p>
<p>I saw you cancelled {plan_name}. Nothing more will be charged, and nothing changes until <strong>{ends_on}</strong>: your apps and your {credits_left} remaining AI credits work until then. After that, the apps go back to their free version.</p>
<p>Could you tell me why? Just hit reply. One line is plenty, for example:</p>
<ul>
<li>I didn't use it enough</li>
<li>An app was missing something I needed</li>
<li>Too expensive for what I got</li>
<li>Something broke</li>
</ul>
<p>I read every answer, and it decides what I build next.</p>
<p>If it's only the price or the amount, you can switch to a smaller plan under <strong>Card, invoices &amp; plan change</strong> on your subscription page. And if you change your mind, the same page has a <strong>Resume plan</strong> button until {ends_on}, and you keep the same licence.</p>
{button}
<p>Mohamed</p>"#,
            button = button(&page, "Manage my subscription"),
        ),
        (false, Lang::Fr) => format!(
            r#"<p>Bonjour,</p>
<p>J'ai vu que vous avez résilié {plan_name}. Plus rien ne sera prélevé, et rien ne change avant le <strong>{ends_on}</strong> : vos apps et vos {credits_left} crédits IA restants fonctionnent jusque-là. Ensuite, les apps reviennent à leur version gratuite.</p>
<p>Pourriez-vous me dire pourquoi ? Répondez simplement à cet e-mail. Une ligne suffit, par exemple :</p>
<ul>
<li>Je ne l'utilisais pas assez</li>
<li>Il manquait quelque chose à une app</li>
<li>Trop cher pour ce que j'en tirais</li>
<li>Quelque chose ne fonctionnait pas</li>
</ul>
<p>Je lis chaque réponse, et c'est ce qui décide de ce que je construis ensuite.</p>
<p>Si c'est seulement une question de prix ou de volume, vous pouvez passer à une formule plus petite sous <strong>Carte, factures et changement de formule</strong> sur votre page d'abonnement. Et si vous changez d'avis, la même page propose un bouton <strong>Reprendre la formule</strong> jusqu'au {ends_on}, et vous gardez la même licence.</p>
{button}
<p>Mohamed</p>"#,
            button = button(&page, "Gérer mon abonnement"),
        ),
        (true, Lang::En) => format!(
            r#"<p>Hi,</p>
<p>Your {plan_name} plan has ended, and nothing more will be charged. The apps are back to their free version, and everything you set up in them is still there.</p>
<p>If you meant to stop, could you tell me why? Just hit reply. One line is plenty, and I read every answer.</p>
<p>If it wasn't on purpose, for example a card that expired, you can subscribe again in a minute.</p>
{button}
<p>Mohamed</p>"#,
            button = button(&page, "See the plans"),
        ),
        (true, Lang::Fr) => format!(
            r#"<p>Bonjour,</p>
<p>Votre formule {plan_name} a pris fin, et plus rien ne sera prélevé. Les apps sont revenues à leur version gratuite, et tout ce que vous y avez configuré est toujours là.</p>
<p>Si vous vouliez arrêter, pourriez-vous me dire pourquoi ? Répondez simplement à cet e-mail. Une ligne suffit, et je lis chaque réponse.</p>
<p>Si ce n'était pas voulu, par exemple une carte expirée, vous pouvez vous réabonner en une minute.</p>
{button}
<p>Mohamed</p>"#,
            button = button(&page, "Voir les formules"),
        ),
    };
    layout(&content, None, lang)
}

// ── Come back ────────────────────────────────────────────────────────────────

pub fn come_back_subject(lang: Lang) -> String {
    match lang {
        Lang::En => "Your apps are waiting for you".into(),
        Lang::Fr => "Vos apps vous attendent".into(),
    }
}

pub fn come_back_body(plan_name: &str, unsubscribe: Option<&str>, lang: Lang) -> String {
    let page = plans_page();
    let content = match lang {
        Lang::En => format!(
            r#"<p>Hi,</p>
<p>Your {plan_name} plan ended a little while ago. Everything you set up in the apps is still there.</p>
<p>Subscribe again whenever you like: it takes a minute, and your AI credits start again straight away.</p>
{button}
<p>Mohamed</p>"#,
            button = button(&page, "See the plans"),
        ),
        Lang::Fr => format!(
            r#"<p>Bonjour,</p>
<p>Votre formule {plan_name} a pris fin il y a quelque temps. Tout ce que vous avez configuré dans les apps est toujours là.</p>
<p>Réabonnez-vous quand vous voulez : cela prend une minute, et vos crédits IA repartent aussitôt.</p>
{button}
<p>Mohamed</p>"#,
            button = button(&page, "Voir les formules"),
        ),
    };
    layout(&content, unsubscribe, lang)
}

// ── Layout ───────────────────────────────────────────────────────────────────

/// mayorana.ch's plain layout, for subscribers who may never have heard of
/// api0. The postal address and opt-out link are only on optional emails.
pub fn layout(content: &str, unsubscribe: Option<&str>, lang: Lang) -> String {
    let unsubscribe = unsubscribe
        .map(|url| {
            let (text, rest) = match lang {
                Lang::En => ("Unsubscribe from these emails", "emails about your subscription are still sent."),
                Lang::Fr => ("Se désabonner de ces e-mails", "les e-mails concernant votre abonnement restent envoyés."),
            };
            format!(
                r#"<br>{POSTAL_ADDRESS}<br><a href="{}" style="color:#64748B">{text}</a> — {rest}"#,
                escape_html(url)
            )
        })
        .unwrap_or_default();
    format!(
        r#"<!DOCTYPE html>
<html lang="{lang}">
<head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"></head>
<body style="margin:0;padding:0;background:#F8FAFC;font-family:Arial,Helvetica,sans-serif">
<div style="max-width:600px;margin:24px auto;background:#fff;border-radius:8px;overflow:hidden;box-shadow:0 1px 3px rgba(0,0,0,0.1)">
  <div style="padding:32px;color:#1E293B;line-height:1.6">{content}</div>
  <div style="padding:16px 32px;background:#F8FAFC;color:#64748B;font-size:12px;text-align:center">
    <a href="https://mayorana.ch" style="color:#475569">mayorana.ch</a>{unsubscribe}
  </div>
</div>
</body>
</html>"#,
        lang = lang.tag(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn a_locale_tag_picks_the_language() {
        assert_eq!(Lang::from_tag(Some("fr")), Lang::Fr);
        assert_eq!(Lang::from_tag(Some(" FR-ch ")), Lang::Fr);
        assert_eq!(Lang::from_tag(Some("en")), Lang::En);
        assert_eq!(Lang::from_tag(Some("de")), Lang::En);
        assert_eq!(Lang::from_tag(Some("frisian")), Lang::En);
        assert_eq!(Lang::from_tag(None), Lang::En);
    }

    #[test]
    fn dates_read_naturally_in_each_language() {
        let first = Utc.with_ymd_and_hms(2026, 11, 1, 12, 0, 0).unwrap();
        let later = Utc.with_ymd_and_hms(2026, 2, 22, 12, 0, 0).unwrap();
        assert_eq!(format_date(first, Lang::En), "1 November 2026");
        assert_eq!(format_date(first, Lang::Fr), "1er novembre 2026");
        assert_eq!(format_date(later, Lang::Fr), "22 février 2026");
    }

    #[test]
    fn a_paid_licence_says_how_to_turn_it_on_and_when_credits_refill() {
        let en = licence_body("Pro", "ABCDE-FGHJK", "*", 2000, Some("22 November 2026"), Lang::En);
        assert!(en.contains("Thanks for subscribing to Pro") && en.contains("ABCDE-FGHJK"));
        assert!(en.contains("every app in your plan") && en.contains("refill on 22 November 2026"));
        assert!(en.contains("https://mayorana.ch/en/apps") && !en.contains("Unsubscribe"));
        let fr = licence_body("Pro", "ABCDE-FGHJK", "gitagent, jobscout", 2000, Some("22 novembre 2026"), Lang::Fr);
        assert!(fr.contains("Merci pour votre abonnement à Pro") && fr.contains("Elle active Pro dans gitagent, jobscout"));
        assert!(fr.contains("renouvelés le 22 novembre 2026") && fr.contains(r#"lang="fr""#));
    }

    #[test]
    fn the_free_licence_is_not_thanked_as_a_subscription() {
        let en = licence_body("Free", "ABCDE-FGHJK", "", 20, None, Lang::En);
        assert!(en.contains("your free licence") && en.contains("20 AI credits"));
        assert!(!en.contains("Thanks for subscribing") && !en.contains("cancel"));
    }

    #[test]
    fn a_cancellation_asks_why_and_offers_the_way_back() {
        let en = cancelled_body("Pro", "22 November 2026", 340, false, Lang::En);
        assert!(en.contains("until <strong>22 November 2026</strong>") && en.contains("340 remaining AI credits"));
        assert!(en.contains("Could you tell me why?") && en.contains("Resume plan") && en.contains("mayorana.ch/pricing"));
        let fr = cancelled_body("Pro", "22 novembre 2026", 340, false, Lang::Fr);
        assert!(fr.contains("Reprendre la formule") && fr.contains("Gérer mon abonnement"));
    }

    #[test]
    fn a_plan_that_already_ended_is_not_blamed_on_the_reader() {
        let en = cancelled_body("Pro", "22 November 2026", 0, true, Lang::En);
        assert!(en.contains("has ended") && en.contains("a card that expired"));
        assert!(!en.contains("I saw you cancelled") && !en.contains("Resume plan"));
        assert_eq!(cancelled_subject("Pro", true, Lang::Fr), "Votre formule Pro a pris fin");
    }

    #[test]
    fn the_come_back_reminder_carries_its_opt_out_in_the_readers_language() {
        let fr = come_back_body("Pro", Some("https://gateway.api0.ai/api/email/unsubscribe?e=a&t=b"), Lang::Fr);
        assert!(fr.contains("Votre formule Pro a pris fin") && fr.contains("Se désabonner de ces e-mails"));
    }
}
