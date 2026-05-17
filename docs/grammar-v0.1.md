# NTS Grammar v0.1 Draft

This is a small readable grammar sketch, not the final parser grammar.

```ebnf
program        = module_decl, import_decl*, item* ;
module_decl    = "module", path ;
import_decl    = "import", path ;

item           = struct_decl | type_alias | fn_decl ;

struct_decl    = "struct", ident, "{", field_decl*, "}" ;
field_decl     = ident, ":", type_expr ;

type_alias     = "type", ident, "=", type_expr ;

fn_decl        = "fn", ident, "(", param_list?, ")", ":", type_expr, block ;
param_list     = param, (",", param)* ;
param          = ident, ":", type_expr ;

block          = "{", stmt*, "}" ;
stmt           = let_stmt
               | if_stmt
               | for_stmt
               | while_stmt
               | match_stmt
               | return_stmt
               | break_stmt
               | continue_stmt
               | expr_stmt ;

let_stmt       = "let", "mut"?, ident, ":", type_expr, "=", expr ;
if_stmt        = "if", expr, block, else_branch? ;
else_branch    = "else", (if_stmt | block) ;

for_stmt       = "for", ident, "in", expr, block ;
while_stmt     = "while", expr, block ;

return_stmt    = "return", expr? ;
break_stmt     = "break" ;
continue_stmt  = "continue" ;
expr_stmt      = expr ;

type_expr      = ident
               | ident, "<", type_expr, (",", type_expr)*, ">" ;

expr           = assignment ;
assignment     = logical_or ;
logical_or     = logical_and, ("||", logical_and)* ;
logical_and    = equality, ("&&", equality)* ;
equality       = comparison, (("==" | "!="), comparison)* ;
comparison     = term, ((">" | ">=" | "<" | "<="), term)* ;
term           = factor, (("+" | "-"), factor)* ;
factor         = unary, (("*" | "/" | "%"), unary)* ;
unary          = ("!" | "-"), unary | primary ;
primary        = literal | ident | call | field_access | "(", expr, ")" ;
```

## Canonical `if else`

```nts
if age >= 18 {
    return 1
} else {
    return 0
}
```

Invalid in NTS:

```nts
if (age >= 18) { return 1 }
if age { return 1 }
if age >= 18 return 1
```
