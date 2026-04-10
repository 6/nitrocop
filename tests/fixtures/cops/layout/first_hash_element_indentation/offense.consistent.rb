# nitrocop-expect: 2:6 Layout/FirstHashElementIndentation: Use 2 (not 6) spaces for indentation of the first element.
# nitrocop-expect: 4:4 Layout/FirstHashElementIndentation: Indent the right brace the same as the start of the line where the left brace is.
foo({
      a: 1,
      b: 2,
    })

func(x: {
  a: 1,
  ^^^^ Layout/FirstHashElementIndentation: Use 2 (not 0) spaces for indentation of the first element.
       b: 2
},
^ Layout/FirstHashElementIndentation: Indent the right brace the same as the parent hash key.
     y: {
       c: 1,
       d: 2
     })
