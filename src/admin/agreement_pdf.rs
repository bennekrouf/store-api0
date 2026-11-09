// src/admin/agreement_pdf.rs
//
// The PDF a customer receives when accepting a legal document: one page saying
// who accepted what and when, then the full text of that version.
//
// Written here rather than in the browser, so the customer's own machine has no
// say in what the record contains. It uses PDF's built-in Helvetica, which covers
// Western European text; any other character is printed as "?".

use chrono::{DateTime, Utc};

const PAGE_WIDTH: f32 = 595.28;
const PAGE_HEIGHT: f32 = 841.89;
const MARGIN: f32 = 56.0;
const LABEL_WIDTH: f32 = 128.0;
const BOTTOM: f32 = MARGIN + 22.0;
const KEEP_WITH_HEADING: f32 = 60.0;

const PROVIDER_NAME: &str = "Mayorana";
const PROVIDER_PLACE: &str = "Saint-Prex, Switzerland";
const PROVIDER_EMAIL: &str = "contact@mayorana.ch";

type Rgb = (f32, f32, f32);
const INK: Rgb = (0.10, 0.11, 0.13);
const MUTED: Rgb = (0.42, 0.45, 0.50);
const ORANGE: Rgb = (1.0, 0.40, 0.0);

/// Helvetica advance widths for ASCII 32..=126, in 1/1000 of the font size.
const REGULAR: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556, 556, 556, 556, 556,
    556, 556, 556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833,
    722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556,
    556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334,
    260, 334, 584,
];
const BOLD: [u16; 95] = [
    278, 333, 474, 556, 556, 889, 722, 238, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556, 556, 556, 556, 556,
    556, 556, 556, 333, 333, 584, 584, 584, 611, 975, 722, 722, 722, 722, 667, 611, 778, 722, 278, 556, 722, 611, 833,
    722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 333, 278, 333, 584, 556, 333, 556, 611, 556, 611,
    556, 333, 611, 611, 278, 278, 556, 278, 889, 611, 611, 611, 611, 389, 556, 333, 611, 556, 778, 556, 556, 500, 389,
    280, 389, 584,
];
const WIDE_DEFAULT: u16 = 667;

#[derive(Clone, Copy)]
enum Font {
    Regular,
    Bold,
}

impl Font {
    fn name(self) -> &'static str {
        match self {
            Font::Regular => "F1",
            Font::Bold => "F2",
        }
    }
    fn table(self) -> &'static [u16; 95] {
        match self {
            Font::Regular => &REGULAR,
            Font::Bold => &BOLD,
        }
    }
}

/// The Windows-1252 byte the built-in fonts use for a character.
fn encode(c: char) -> u8 {
    match c as u32 {
        code @ (0x20..=0x7E | 0xA0..=0xFF) => code as u8,
        _ => match c {
            '€' => 0x80,
            '…' => 0x85,
            '‘' => 0x91,
            '’' => 0x92,
            '“' => 0x93,
            '”' => 0x94,
            '•' => 0x95,
            '–' => 0x96,
            '—' => 0x97,
            _ => b'?',
        },
    }
}

fn char_width(c: char, font: Font, size: f32) -> f32 {
    let byte = encode(c);
    let units = if (0x20..=0x7E).contains(&byte) { font.table()[(byte - 0x20) as usize] } else { WIDE_DEFAULT };
    units as f32 * size / 1000.0
}

fn text_width(s: &str, font: Font, size: f32) -> f32 {
    s.chars().map(|c| char_width(c, font, size)).sum()
}

/// Breaks at spaces, and inside a word only when the word alone is too long.
fn wrap(text: &str, font: Font, size: f32, width: f32) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.lines() {
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if text_width(&candidate, font, size) <= width {
                line = candidate;
                continue;
            }
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            for c in word.chars() {
                if text_width(&line, font, size) + char_width(c, font, size) > width {
                    lines.push(std::mem::take(&mut line));
                }
                line.push(c);
            }
        }
        lines.push(line);
    }
    lines
}

struct Pages {
    done: Vec<String>,
    current: String,
    y: f32,
}

impl Pages {
    fn new() -> Self {
        Pages { done: Vec::new(), current: String::new(), y: PAGE_HEIGHT - MARGIN }
    }

    fn new_page(&mut self) {
        self.done.push(std::mem::take(&mut self.current));
        self.y = PAGE_HEIGHT - MARGIN;
    }

    fn room_for(&mut self, height: f32) {
        if self.y - height < BOTTOM {
            self.new_page();
        }
    }

    fn draw(&mut self, text: &str, x: f32, y: f32, font: Font, size: f32, color: Rgb) {
        let hex: String = text.chars().map(|c| format!("{:02X}", encode(c))).collect();
        self.current.push_str(&format!(
            "BT /{} {size:.2} Tf {:.3} {:.3} {:.3} rg {x:.2} {y:.2} Td <{hex}> Tj ET\n",
            font.name(),
            color.0,
            color.1,
            color.2,
        ));
    }

    fn paragraph(&mut self, text: &str, x: f32, font: Font, size: f32, color: Rgb) {
        let height = size * 1.4;
        for line in wrap(text, font, size, PAGE_WIDTH - MARGIN - x) {
            self.room_for(height);
            self.y -= height;
            let y = self.y;
            self.draw(&line, x, y, font, size, color);
        }
    }

    fn gap(&mut self, height: f32) {
        self.y -= height;
    }

    fn rule(&mut self) {
        self.current.push_str(&format!(
            "0.85 0.87 0.90 RG 0.6 w {MARGIN:.2} {y:.2} m {right:.2} {y:.2} l S\n",
            y = self.y,
            right = PAGE_WIDTH - MARGIN,
        ));
    }

    fn section(&mut self, title: &str) {
        self.gap(13.0);
        self.paragraph(title, MARGIN, Font::Bold, 11.0, INK);
        self.gap(4.0);
        self.rule();
        self.gap(3.0);
    }

    fn row(&mut self, label: &str, value: &str) {
        if value.trim().is_empty() {
            return;
        }
        let top = self.y;
        self.y -= 13.3;
        let y = self.y;
        self.draw(label, MARGIN, y, Font::Regular, 9.5, MUTED);
        self.y = top;
        self.paragraph(value, MARGIN + LABEL_WIDTH, Font::Regular, 9.5, INK);
        self.gap(2.5);
    }

    fn bullet(&mut self, text: &str, size: f32) {
        self.room_for(size * 1.4);
        let y = self.y - size * 1.4;
        self.draw("•", MARGIN + 3.0, y, Font::Regular, size, INK);
        self.paragraph(text, MARGIN + 14.0, Font::Regular, size, INK);
        self.gap(2.0);
    }

    fn finish(mut self, reference: &str) -> Vec<String> {
        self.done.push(self.current);
        let total = self.done.len();
        let mut pages = self.done;
        for (i, page) in pages.iter_mut().enumerate() {
            let mut footer = Pages { done: Vec::new(), current: String::new(), y: 0.0 };
            footer.draw(&format!("api0 - Record {reference}"), MARGIN, 34.0, Font::Regular, 8.0, MUTED);
            let number = format!("Page {} of {}", i + 1, total);
            let x = PAGE_WIDTH - MARGIN - text_width(&number, Font::Regular, 8.0);
            footer.draw(&number, x, 34.0, Font::Regular, 8.0, MUTED);
            page.push_str(&footer.current);
        }
        pages
    }
}

pub struct Agreement<'a> {
    pub reference: &'a str,
    pub document_title: &'a str,
    pub version: &'a str,
    pub url: &'a str,
    pub text: &'a str,
    pub sha256: &'a str,
    pub accepted_at: DateTime<Utc>,
    pub accepted_by: &'a str,
    pub legal_name: &'a str,
    pub registered_address: &'a str,
    pub country: &'a str,
    pub company_number: &'a str,
    pub vat_id: &'a str,
    pub tenant_name: &'a str,
    pub tenant_id: &'a str,
}

fn first_page(p: &mut Pages, a: &Agreement) {
    p.draw("api0", MARGIN, PAGE_HEIGHT - MARGIN - 18.0, Font::Bold, 24.0, ORANGE);
    p.gap(42.0);
    p.paragraph("Electronic acceptance record", MARGIN, Font::Bold, 18.0, INK);
    p.gap(3.0);
    p.paragraph(&format!("{}, version {}", a.document_title, a.version), MARGIN, Font::Regular, 11.0, MUTED);

    p.section("Agreement");
    p.row("Document", a.document_title);
    p.row("Version", a.version);
    p.row("Published at", a.url);
    p.row("Accepted on", &a.accepted_at.format("%-d %B %Y at %H:%M:%S UTC").to_string());
    p.row("Text fingerprint", &format!("SHA-256 {}", a.sha256));

    p.section("Provider");
    p.row("Name", PROVIDER_NAME);
    p.row("Address", PROVIDER_PLACE);
    p.row("Email", PROVIDER_EMAIL);

    p.section("Customer");
    p.row("Legal name", a.legal_name);
    p.row("Registered address", a.registered_address);
    p.row("Country", a.country);
    p.row("Company number", a.company_number);
    p.row("VAT number", a.vat_id);
    p.row("Workspace", &format!("{} ({})", a.tenant_name, a.tenant_id));

    p.section("Accepted by");
    p.row("Account", a.accepted_by);
    p.gap(2.0);
    p.paragraph("Signed in to the api0 dashboard, this person ticked boxes confirming that:", MARGIN, Font::Regular, 9.5, INK);
    p.gap(3.0);
    p.bullet("they accept this document, in this version, for the customer named above;", 9.5);
    p.bullet("these details are accurate, and they are allowed to accept this document for the customer;", 9.5);
    p.bullet("the customer uses api0 for its business, not as a private consumer.", 9.5);
    p.gap(7.0);
    p.paragraph(
        "This records an electronic acceptance. It is not a handwritten or qualified electronic signature. \
         The company details were declared by the customer and have not been verified by the provider. \
         The full text of the accepted version follows.",
        MARGIN,
        Font::Regular,
        8.5,
        MUTED,
    );
}

fn full_text(p: &mut Pages, a: &Agreement) {
    p.new_page();
    let mut lines = a.text.lines().filter(|l| !l.trim().is_empty());
    if let Some(title) = lines.next() {
        p.paragraph(title, MARGIN, Font::Bold, 16.0, INK);
        p.gap(4.0);
    }
    for line in lines {
        if let Some(heading) = line.strip_prefix("## ") {
            p.room_for(KEEP_WITH_HEADING);
            p.gap(6.0);
            p.paragraph(heading, MARGIN, Font::Bold, 10.0, INK);
            p.gap(2.0);
        } else if let Some(heading) = line.strip_prefix("# ") {
            p.room_for(KEEP_WITH_HEADING);
            p.gap(11.0);
            p.paragraph(heading, MARGIN, Font::Bold, 12.0, INK);
            p.gap(3.0);
        } else if let Some(item) = line.strip_prefix("- ") {
            p.bullet(item, 9.5);
        } else {
            p.paragraph(line, MARGIN, Font::Regular, 9.5, INK);
            p.gap(4.0);
        }
    }
}

/// A file of objects, then the table saying where each one starts.
fn assemble(pages: &[String], title: &str) -> Vec<u8> {
    let first_page_object = 5;
    let kids: Vec<String> = (0..pages.len()).map(|i| format!("{} 0 R", first_page_object + i * 2)).collect();
    let title_hex: String = title.chars().map(|c| format!("{:02X}", encode(c))).collect();

    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), pages.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_string(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>".to_string(),
    ];
    for (i, content) in pages.iter().enumerate() {
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {PAGE_WIDTH} {PAGE_HEIGHT}] \
             /Resources << /Font << /F1 3 0 R /F2 4 0 R >> >> /Contents {} 0 R >>",
            first_page_object + i * 2 + 1,
        ));
        objects.push(format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()));
    }
    objects.push(format!("<< /Title <{title_hex}> /Producer (api0) >>"));
    let info = objects.len();

    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (i, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", i + 1).as_bytes());
    }
    let table = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {} /Root 1 0 R /Info {info} 0 R >>\nstartxref\n{table}\n%%EOF\n", objects.len() + 1)
            .as_bytes(),
    );
    out
}

pub fn render(a: &Agreement) -> Vec<u8> {
    let mut p = Pages::new();
    first_page(&mut p, a);
    full_text(&mut p, a);
    let pages = p.finish(a.reference);
    assemble(&pages, &format!("{} acceptance record", a.document_title))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample<'a>(text: &'a str) -> Agreement<'a> {
        Agreement {
            reference: "A-7",
            document_title: "Terms of Service",
            version: "2025-10-27",
            url: "https://api0.ai/terms",
            text,
            sha256: "abc123",
            accepted_at: DateTime::parse_from_rfc3339("2026-11-09T10:00:00Z").unwrap().with_timezone(&Utc),
            accepted_by: "owner@example.com",
            legal_name: "Société Exemple SA",
            registered_address: "1 Rue Exemple\n1200 Genève",
            country: "CH",
            company_number: "",
            vat_id: "CHE123456789MWST",
            tenant_name: "Example",
            tenant_id: "t1",
        }
    }

    fn hex_of(text: &str) -> String {
        text.chars().map(|c| format!("{:02X}", encode(c))).collect()
    }

    #[test]
    fn the_file_table_points_at_each_object() {
        let pdf = render(&sample("Title\n# One\nBody"));
        let text = String::from_utf8(pdf.clone()).unwrap();
        assert!(text.starts_with("%PDF-1.4"));
        assert!(text.ends_with("%%EOF\n"));

        let table = text.find("xref\n").unwrap();
        let start: usize = text.rsplit("startxref\n").next().unwrap().lines().next().unwrap().parse().unwrap();
        assert_eq!(start, table);
        for (i, line) in text[table..].lines().skip(3).take_while(|l| l.ends_with(" n ")).enumerate() {
            let offset: usize = line[..10].parse().unwrap();
            assert!(text[offset..].starts_with(&format!("{} 0 obj", i + 1)), "object {}", i + 1);
        }
    }

    #[test]
    fn the_record_names_the_parties_and_carries_the_full_text() {
        let text = String::from_utf8(render(&sample("Title\n# Heading\nThe very last clause"))).unwrap();
        for expected in ["Société Exemple SA", "owner@example.com", "2025-10-27", "9 November 2026 at 10:00:00 UTC", "The very last clause", "Mayorana"] {
            assert!(text.contains(&hex_of(expected)), "{expected}");
        }
    }

    #[test]
    fn an_empty_company_number_is_left_out() {
        let text = String::from_utf8(render(&sample("Title"))).unwrap();
        assert!(!text.contains(&hex_of("Company number")));
    }

    #[test]
    fn a_long_document_runs_over_several_pages_and_each_is_numbered() {
        let long = format!("Title\n{}", "A paragraph of the agreement that takes room.\n".repeat(400));
        let text = String::from_utf8(render(&sample(&long))).unwrap();
        assert!(text.contains(&hex_of("Page 2 of")));
        let count = text.matches("/Type /Page /Parent").count();
        assert!(count > 3);
        assert!(text.contains(&format!("/Count {count}")));
    }

    #[test]
    fn nothing_is_drawn_past_the_right_margin() {
        for line in wrap(&"word ".repeat(200), Font::Regular, 9.5, 300.0) {
            assert!(text_width(&line, Font::Regular, 9.5) <= 300.0);
        }
        let unbroken = wrap(&"x".repeat(500), Font::Regular, 9.5, 300.0);
        assert!(unbroken.len() > 1);
        assert!(unbroken.iter().all(|l| text_width(l, Font::Regular, 9.5) <= 300.0));
    }

    #[test]
    fn characters_the_font_lacks_cannot_break_the_file() {
        assert_eq!(encode('é'), 0xE9);
        assert_eq!(encode('’'), 0x92);
        assert_eq!(encode('株'), b'?');
        assert_eq!(encode('('), b'(');
        let text = String::from_utf8(render(&sample("(株) ) \\ Title"))).unwrap();
        assert!(text.contains("%%EOF"));
    }
}
