# coding: US-ASCII
# Valid UTF-8 hex escape in regex — does NOT crash RuboCop, offenses detected
/\A\xef\xbb\xbf/ =~ source

content = content.encode(encoding,
                         :invalid => :replace,
                         ^^^^^^^^ Style/HashSyntax: Use the new Ruby 1.9 hash syntax.
                         :undef => :replace,
                         ^^^^^^ Style/HashSyntax: Use the new Ruby 1.9 hash syntax.
                         :replace => "?")
                         ^^^^^^^^ Style/HashSyntax: Use the new Ruby 1.9 hash syntax.

String.new text, encoding: encoding
                           ^^^^^^^^ Style/HashSyntax: Omit the hash value.

picture.image_file.attach(
  io: acc.image_file.open,
  filename:,
  ^ Style/HashSyntax: Do not mix explicit and implicit hash values. Include the hash value.
  content_type:,
  ^ Style/HashSyntax: Do not mix explicit and implicit hash values. Include the hash value.
  identify: false,
  metadata: {
    width: 1,
    height: 1
  }
)

render :index, locals: { calendar: calendar }
                                   ^ Style/HashSyntax: Omit the hash value.
