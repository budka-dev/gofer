(function_item
  name: (identifier) @name
) @function

(struct_item
  name: (type_identifier) @name
) @struct

(enum_item
  name: (type_identifier) @name
) @enum

(impl_item
  type: (type_identifier) @name
) @impl

(impl_item
  type: (generic_type
    type: (type_identifier) @name
  )
) @impl

(trait_item
  name: (type_identifier) @name
) @trait

(const_item
  name: (identifier) @name
) @const

(static_item
  name: (identifier) @name
) @static

(mod_item
  name: (identifier) @name
) @module

(macro_definition
  name: (identifier) @name
) @macro

(type_item
  name: (type_identifier) @name
) @type_alias
