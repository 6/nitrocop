# coding: US-ASCII
# Valid UTF-8 hex escape in regex — does NOT crash RuboCop, offenses detected
/\A\xef\xbb\xbf/ =~ source

String.new text, encoding: encoding
                 ^^^^^^^^^ Style/HashSyntax: Use hash rockets syntax.
                           ^^^^^^^^ Style/HashSyntax: Omit the hash value.

LegacyPageUrl.joins(:page).where(
  :urlname => urlname,
  Page.table_name => {
    language_id: Current.language.id
    ^ Style/HashSyntax: Use hash rockets syntax.
  }
)

GiftForm.new(gift:           gift,
             ^ Style/HashSyntax: Use hash rockets syntax.
                             ^ Style/HashSyntax: Omit the hash value.
             contributions:  current_user.unspent_contributions,
             ^ Style/HashSyntax: Use hash rockets syntax.
             giftable_dates: Gift.giftable_dates)
             ^ Style/HashSyntax: Use hash rockets syntax.
