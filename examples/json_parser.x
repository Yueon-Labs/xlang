module main

// json_parser — a recursive-descent JSON parser + serializer.
// Showcases: recursive enums (Json containing Vec<Json>), mut self (parser
// position), match (traversal), Vec (arrays/objects), char literals.
// Run: xlangc run examples/json_parser.x

struct KV {
    key: String
    val: Json
}

enum Json {
    Null
    Bool(bool)
    Num(i32)
    Str(String)
    Arr(Vec<Json>)
    Obj(Vec<KV>)
}

struct Parser {
    input: String
    pos: i32
}

impl Parser {
    fn new(input: String): Parser {
        return Parser { input: input, pos: 0 }
    }
    fn at(self: Parser): i32 {
        if self.pos >= str_len(self.input) { return -1 }
        return str_char_at(self.input, self.pos)
    }
    fn skip_ws(mut self: Parser): i32 {
        while self.pos < str_len(self.input) {
            let c: i32 = str_char_at(self.input, self.pos)
            if c == ' ' || c == '\t' || c == '\n' || c == '\r' {
                self.pos += 1
            } else {
                break
            }
        }
        return 0
    }
    fn expect(mut self: Parser, ch: i32): i32 {
        self.skip_ws()
        if self.at() != ch {
            print_str("parse error: expected ")
            print_i32(ch)
            print_str(" at pos ")
            print_i32(self.pos)
            print_str("\n")
            return -1
        }
        self.pos += 1
        return 0
    }
    fn parse_value(mut self: Parser): Json {
        self.skip_ws()
        let c: i32 = self.at()
        if c == '{' { return self.parse_obj() }
        if c == '[' { return self.parse_arr() }
        if c == '"' {
            let s: String = self.parse_raw_string()
            return Str(s)
        }
        if c == 't' {
            self.pos += 4
            return Bool(true)
        }
        if c == 'f' {
            self.pos += 5
            return Bool(false)
        }
        if c == 'n' {
            self.pos += 4
            return Null
        }
        return self.parse_num()
    }
    fn parse_raw_string(mut self: Parser): String {
        self.pos += 1
        let start: i32 = self.pos
        while self.pos < str_len(self.input) {
            if str_char_at(self.input, self.pos) == '"' { break }
            self.pos += 1
        }
        let s: String = str_slice(self.input, start, self.pos)
        self.pos += 1
        return s
    }
    fn parse_num(mut self: Parser): Json {
        let start: i32 = self.pos
        if self.at() == '-' { self.pos += 1 }
        while self.pos < str_len(self.input) {
            let c: i32 = str_char_at(self.input, self.pos)
            if c >= '0' && c <= '9' { self.pos += 1 } else { break }
        }
        let n: i32 = str_to_int(str_slice(self.input, start, self.pos))
        return Num(n)
    }
    fn parse_arr(mut self: Parser): Json {
        self.pos += 1
        let items: Vec<Json> = vec_new()
        self.skip_ws()
        if self.at() == ']' {
            self.pos += 1
            return Arr(items)
        }
        while true {
            let v: Json = self.parse_value()
            items.push(v)
            self.skip_ws()
            if self.at() == ',' { self.pos += 1 } else { break }
        }
        self.expect(']')
        return Arr(items)
    }
    fn parse_obj(mut self: Parser): Json {
        self.pos += 1
        let entries: Vec<KV> = vec_new()
        self.skip_ws()
        if self.at() == '}' {
            self.pos += 1
            return Obj(entries)
        }
        while true {
            self.skip_ws()
            let key: String = self.parse_raw_string()
            self.expect(':')
            let val: Json = self.parse_value()
            entries.push(KV { key: key, val: val })
            self.skip_ws()
            if self.at() == ',' { self.pos += 1 } else { break }
        }
        self.expect('}')
        return Obj(entries)
    }
}

fn serialize(j: Json): String {
    sb_new()
    serialize_into(j)
    return str_slice(sb_str(), 0, str_len(sb_str()))
}

fn serialize_into(j: Json): i32 {
    match j {
        Null => { sb_push("null") }
        Bool(b) => {
            if b { sb_push("true") } else { sb_push("false") }
        }
        Num(n) => { sb_push(int_to_str(n)) }
        Str(s) => {
            sb_push("\"")
            sb_push(s)
            sb_push("\"")
        }
        Arr(items) => {
            sb_push("[")
            let n: i32 = vec_len(items)
            let mut i: i32 = 0
            while i < n {
                if i > 0 { sb_push(", ") }
                serialize_into(items[i])
                i += 1
            }
            sb_push("]")
        }
        Obj(entries) => {
            sb_push("{")
            let n: i32 = vec_len(entries)
            let mut i: i32 = 0
            while i < n {
                if i > 0 { sb_push(", ") }
                sb_push("\"")
                sb_push(entries[i].key)
                sb_push("\": ")
                serialize_into(entries[i].val)
                i += 1
            }
            sb_push("}")
        }
    }
    return 0
}

fn main(): i32 {
    let input: String = "{\"name\":\"xlang\",\"year\":2026,\"nested\":{\"ok\":true,\"items\":[1,2,3]},\"tags\":[\"systems\",\"language\"]}"
    let p: Parser = Parser { input: input, pos: 0 }
    let j: Json = p.parse_value()
    let out: String = serialize(j)
    print_str(out)
    print_str("\n")
    assert(str_len(out) > 0)
    return 0
}
