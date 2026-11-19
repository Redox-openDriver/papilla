papilla-frame-dimensions = недопустимый размер кадра { $width }×{ $height }
papilla-frame-size = в кадре { $got } пикселей, ожидалось { $expected }
papilla-frame-mask = в маске { $got } элементов, ожидалось { $expected }
papilla-frame-profile = кадр { $got } не соответствует профилю сенсора ({ $expected })

papilla-storage-not-a-gallery = это не файл галереи papilla
papilla-storage-version = неподдерживаемая версия формата галереи { $version }
papilla-storage-profile = галерея снята другим сенсором (профиль { $profile })
papilla-storage-revision = галерея создана с другими параметрами распознавания (ревизия { $found }, нужна { $expected }) — зарегистрируй палец заново
papilla-storage-binding = галерея привязана к другому экземпляру или варианту сенсора
papilla-storage-corrupt = файл галереи повреждён

papilla-vault-not-sealed = данные не зашифрованы хранилищем papilla
papilla-vault-version = неподдерживаемая версия хранилища { $version }
papilla-vault-algorithm = данные зашифрованы другим алгоритмом ({ $found }), эта сборка использует { $expected }
papilla-vault-rejected = проверка целостности не прошла: другой ключ, другой владелец или файл изменён
papilla-vault-random = системный генератор случайных чисел недоступен

papilla-profile-name = профиль сенсора: имя должно состоять из строчных латинских букв, цифр и дефисов (до 64 символов)
papilla-profile-revision = профиль сенсора: ревизия должна быть не меньше 1
papilla-profile-dimensions = профиль сенсора: недопустимый размер кадра
papilla-profile-dpi = профиль сенсора: недопустимое разрешение
papilla-profile-detector = профиль сенсора: параметры детектора вне безопасных границ
papilla-profile-matching = профиль сенсора: параметры сопоставления вне безопасных границ
papilla-profile-accept-score = профиль сенсора: порог допуска слишком низкий
papilla-profile-enrolling = профиль сенсора: противоречивые параметры регистрации

papilla-enroll-inconsistent = из { $total } касаний только { $kept } подтверждают друг друга — этого мало: возможно, прикладывались разные пальцы или палец каждый раз ложился в новое место; регистрация не сохранена
papilla-enroll-incomplete = регистрация не закончена: касаний { $touches }, подтверждённых { $kept }
papilla-enroll-featureless = в кадре слишком мало особых точек ({ $points }, нужно хотя бы { $needed })
papilla-enroll-misfit = шаблон касания не подходит к профилю сенсора (другой размер кадра или слишком много точек)
papilla-enroll-fixed = { $still } из { $touches } касаний показали на тех же местах то же, что половина прежних или больше: либо палец каждый раз ложится одинаково, либо под ним рисунок, который не двигается, — грязь, плёнка или след на сканере. Протри сканер и начни регистрацию заново, каждый раз прикладывая палец заметно иначе
