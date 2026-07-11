; Calls
(call_expression
  function: (identifier) @call
)

(call_expression
  function: (field_expression
    field: (field_identifier) @call
  )
)

(call_expression
  function: (scoped_identifier
    name: (identifier) @call
  )
)

(macro_invocation
  macro: (identifier) @call
)

; Imports
(use_declaration
  argument: (scoped_identifier
    name: (identifier) @import
  )
)

(use_declaration
  argument: (identifier) @import
)

; Types
(type_identifier) @type_usage

; Trait inheritance / impl Trait for T
(impl_item
  trait: (type_identifier) @inherit
)

(impl_item
  trait: (generic_type
    type: (type_identifier) @inherit
  )
)
