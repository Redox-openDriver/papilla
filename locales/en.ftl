papilla-frame-dimensions = unsupported frame size { $width }×{ $height }
papilla-frame-size = the frame has { $got } pixels, expected { $expected }
papilla-frame-mask = the mask has { $got } elements, expected { $expected }
papilla-frame-profile = frame { $got } does not fit the sensor profile ({ $expected })

papilla-storage-not-a-gallery = not a papilla gallery file
papilla-storage-version = unsupported gallery format version { $version }
papilla-storage-profile = the gallery was made with another sensor (profile { $profile })
papilla-storage-revision = the gallery was made with other recognition parameters (revision { $found }, need { $expected }) — enroll the finger again
papilla-storage-binding = the gallery is bound to another unit or variant of the sensor
papilla-storage-corrupt = the gallery file is damaged

papilla-vault-not-sealed = the data is not sealed by the papilla vault
papilla-vault-version = unsupported vault version { $version }
papilla-vault-algorithm = the data is sealed with another algorithm ({ $found }), this build uses { $expected }
papilla-vault-rejected = integrity check failed: another key, another owner or a modified file
papilla-vault-random = the system random number generator is not available

papilla-profile-name = sensor profile: the name must consist of lowercase Latin letters, digits and hyphens (up to 64 characters)
papilla-profile-revision = sensor profile: the revision must be at least 1
papilla-profile-dimensions = sensor profile: unsupported frame size
papilla-profile-dpi = sensor profile: unsupported resolution
papilla-profile-detector = sensor profile: detector parameters are outside the safe bounds
papilla-profile-matching = sensor profile: matching parameters are outside the safe bounds
papilla-profile-accept-score = sensor profile: the acceptance threshold is too low
papilla-profile-enrolling = sensor profile: inconsistent enrollment parameters

papilla-enroll-inconsistent = only { $kept } of { $total } touches confirm each other — too few: different fingers may have been used, or the finger landed on a new spot every time; nothing was saved
papilla-enroll-incomplete = enrollment is not finished: { $touches } touches, { $kept } confirmed
papilla-enroll-featureless = too few feature points in the frame ({ $points }, at least { $needed } are needed)
papilla-enroll-misfit = the touch template does not fit the sensor profile (another frame size or too many points)
papilla-enroll-fixed = { $still } of { $touches } touches showed the same thing in the same places as half of the earlier ones or more: either the finger lands the same way every time, or there is a picture under it that does not move — dirt, a film or a print left on the reader. Wipe the reader and enroll again, placing the finger noticeably differently each time
