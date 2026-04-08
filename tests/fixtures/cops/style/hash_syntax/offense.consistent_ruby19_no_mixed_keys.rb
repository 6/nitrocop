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
