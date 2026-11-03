//! Rendering and user actions for the desktop interface.
use super::*;

impl App {
    pub(super) fn login_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, accent: Color32) {
        Self::heading(
            ui,
            "01 / CONNECTION",
            "Ваша музыка — дома",
            "Подключите свой сервер Navidrome",
            accent,
        );
        ui.label("Адрес сервера");
        ui.add(egui::TextEdit::singleline(&mut self.server).desired_width(400.0));
        ui.add_space(8.0);
        ui.label("Логин");
        ui.add(egui::TextEdit::singleline(&mut self.username).desired_width(400.0));
        ui.add_space(8.0);
        ui.label("Пароль");
        ui.add(
            egui::TextEdit::singleline(&mut self.password)
                .password(true)
                .desired_width(400.0),
        );
        ui.add_space(15.0);
        if ui
            .add_enabled(!self.busy, egui::Button::new("ПОДКЛЮЧИТЬСЯ  →"))
            .clicked()
        {
            self.connect(ctx);
        }
        ui.add_space(15.0);
        ui.label(
            RichText::new("Данные входа хранятся только до закрытия приложения.")
                .size(12.0)
                .color(Color32::GRAY),
        );
    }
    pub(super) fn library_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, accent: Color32) {
        Self::heading(
            ui,
            "02 / LIBRARY",
            "Медиатека",
            "Ваша коллекция на Navidrome",
            accent,
        );
        ui.horizontal(|ui| {
            if ui
                .selectable_label(self.library_view == LibraryView::Songs, "Треки")
                .clicked()
            {
                self.library_view = LibraryView::Songs;
            }
            if ui
                .add_enabled(
                    !self.busy,
                    egui::Button::new("Альбомы").selected(self.library_view == LibraryView::Albums),
                )
                .clicked()
            {
                self.library_view = LibraryView::Albums;
                self.albums.clear();
                self.album_artist = None;
                self.album_offset = 0;
                let c = self.client.clone().unwrap();
                self.spawn(ctx, move || Message::Albums(0, None, c.albums(0, 50)));
            }
            if ui
                .add_enabled(
                    !self.busy,
                    egui::Button::new("Исполнители")
                        .selected(self.library_view == LibraryView::Artists),
                )
                .clicked()
            {
                self.library_view = LibraryView::Artists;
                let c = self.client.clone().unwrap();
                self.spawn(ctx, move || Message::Artists(c.artists()));
            }
        });
        ui.add_space(10.0);
        if self.library_view == LibraryView::Albums {
            if let Some(artist) = &self.album_artist {
                ui.label(format!("Альбомы: {artist}"));
            }
            for album in self.albums.clone() {
                let label = format!(
                    "{} — {}  ·  {} треков",
                    album.artist, album.name, album.song_count
                );
                if ui
                    .add_enabled(!self.busy, egui::Button::new(label))
                    .clicked()
                {
                    let c = self.client.clone().unwrap();
                    let id = album.id.clone();
                    self.open_playlist = None;
                    self.library_view = LibraryView::Songs;
                    self.spawn(ctx, move || Message::Songs(c.album_songs(&id)));
                }
            }
            if self.album_artist.is_none() {
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            !self.busy && self.album_offset > 0,
                            egui::Button::new("← Назад"),
                        )
                        .clicked()
                    {
                        let offset = self.album_offset.saturating_sub(50);
                        let c = self.client.clone().unwrap();
                        self.spawn(ctx, move || {
                            Message::Albums(offset, None, c.albums(offset, 50))
                        });
                    }
                    ui.label(format!("Страница {}", self.album_offset / 50 + 1));
                    if ui
                        .add_enabled(
                            !self.busy && self.albums.len() == 50,
                            egui::Button::new("Далее →"),
                        )
                        .clicked()
                    {
                        let offset = self.album_offset + 50;
                        let c = self.client.clone().unwrap();
                        self.spawn(ctx, move || {
                            Message::Albums(offset, None, c.albums(offset, 50))
                        });
                    }
                });
            }
            return;
        }
        if self.library_view == LibraryView::Artists {
            ui.add(
                egui::TextEdit::singleline(&mut self.artist_filter)
                    .hint_text("Фильтр исполнителей"),
            );
            let filter = self.artist_filter.to_lowercase();
            let matching: Vec<usize> = self
                .artists
                .iter()
                .enumerate()
                .filter(|(_, artist)| {
                    filter.is_empty() || artist.name.to_lowercase().contains(&filter)
                })
                .map(|(index, _)| index)
                .collect();
            let mut chosen = None;
            let row_height = ui.spacing().interact_size.y + ui.spacing().item_spacing.y;
            egui::ScrollArea::vertical()
                .id_salt("artist_list")
                .max_height(500.0)
                .show_rows(ui, row_height, matching.len(), |ui, rows| {
                    for row in rows {
                        let artist = &self.artists[matching[row]];
                        let label = format!("{}  ·  {} альбомов", artist.name, artist.album_count);
                        if ui
                            .add_enabled(!self.busy, egui::Button::new(label))
                            .clicked()
                        {
                            chosen = Some((artist.id.clone(), artist.name.clone()));
                        }
                    }
                });
            if let Some((id, name)) = chosen {
                let c = self.client.clone().unwrap();
                self.library_view = LibraryView::Albums;
                self.albums.clear();
                self.album_artist = Some(name.clone());
                self.spawn(ctx, move || {
                    Message::Albums(0, Some(name), c.artist_albums(&id))
                });
            }
            return;
        }
        ui.horizontal(|ui| {
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.query)
                    .hint_text("Поиск треков и исполнителей...")
                    .desired_width(300.0),
            );
            if ui
                .add_enabled(!self.busy, egui::Button::new("Найти"))
                .clicked()
                || (response.lost_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    && !self.busy)
            {
                self.load_songs(ctx, Some(self.query.clone()));
            }
            if ui
                .add_enabled(!self.busy, egui::Button::new("Случайные"))
                .clicked()
            {
                self.load_songs(ctx, None);
            }
            if ui
                .add_enabled(!self.busy, egui::Button::new("Избранное"))
                .clicked()
            {
                let c = self.client.clone().unwrap();
                self.open_playlist = None;
                self.spawn(ctx, move || Message::Songs(c.favorite_songs()));
            }
        });
        if ui
            .add_enabled(
                self.selected.is_some(),
                egui::Button::new("+ В конец очереди"),
            )
            .clicked()
        {
            if let Some(song) = self.selected.clone() {
                self.queue.push(song);
                self.status = "Трек добавлен в очередь".into();
                self.schedule_prefetch();
            }
        }
        ui.horizontal(|ui| {
            for (favorite, label) in [(true, "♡ В избранное"), (false, "Убрать из избранного")]
            {
                if ui
                    .add_enabled(
                        !self.busy && self.selected.is_some(),
                        egui::Button::new(label),
                    )
                    .clicked()
                {
                    let c = self.client.clone().unwrap();
                    let id = self.selected.as_ref().unwrap().id.clone();
                    self.spawn(ctx, move || {
                        Message::FavoriteUpdated(c.set_favorite(&id, favorite), favorite)
                    });
                }
            }
        });
        if let Some(selected) = self.selected.clone() {
            ui.horizontal(|ui| {
                ui.label("Оценка:");
                for rating in 1..=5 {
                    let symbol = if selected.user_rating.unwrap_or(0) >= rating {
                        "★"
                    } else {
                        "☆"
                    };
                    if ui
                        .add_enabled(!self.busy, egui::Button::new(symbol))
                        .clicked()
                    {
                        let c = self.client.clone().unwrap();
                        let id = selected.id.clone();
                        self.spawn(ctx, move || {
                            let result = c.rate_song(&id, rating);
                            Message::Rated(id, rating, result)
                        });
                    }
                }
                if ui
                    .add_enabled(
                        !self.busy && selected.user_rating.unwrap_or(0) > 0,
                        egui::Button::new("Сбросить"),
                    )
                    .clicked()
                {
                    let c = self.client.clone().unwrap();
                    let id = selected.id.clone();
                    self.spawn(ctx, move || {
                        let result = c.rate_song(&id, 0);
                        Message::Rated(id, 0, result)
                    });
                }
            });
        }
        ui.add_space(15.0);
        self.song_list(ui, ctx);
    }
    pub(super) fn mix_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, accent: Color32) {
        Self::heading(
            ui,
            "03 / DISCOVER",
            "Миксы",
            "Подборки по треку, настроению и исполнителю",
            accent,
        );
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut self.mix_mode, MixMode::Navidrome, "Navidrome");
            ui.selectable_value(&mut self.mix_mode, MixMode::Wave, "Волна из избранного");
            ui.selectable_value(&mut self.mix_mode, MixMode::Track, "По треку · AudioMuse");
            ui.selectable_value(&mut self.mix_mode, MixMode::Mood, "По настроению");
            ui.selectable_value(&mut self.mix_mode, MixMode::Artist, "По исполнителю");
            ui.selectable_value(&mut self.mix_mode, MixMode::Genre, "По жанру");
        });
        ui.add_space(10.0);
        if self.mix_mode != MixMode::Navidrome
            && self.mix_mode != MixMode::Wave
            && self.mix_mode != MixMode::Genre
            && self.muse.is_none()
        {
            ui.label("Подключите AudioMuse-AI в настройках.");
        } else {
            match self.mix_mode {
                MixMode::Navidrome | MixMode::Track => {
                    ui.label(
                        self.selected
                            .as_ref()
                            .map(|s| format!("Основа: {} — {}", s.artist, s.title))
                            .unwrap_or_else(|| "Сначала выберите трек в медиатеке".into()),
                    );
                }
                MixMode::Wave => {
                    ui.label("Подборка из избранных треков и похожей музыки. Нужен источник похожих треков в Navidrome.");
                }
                MixMode::Mood => {
                    ui.horizontal(|ui| {
                        ui.label("Настроение:");
                        egui::ComboBox::from_id_salt("mood")
                            .selected_text(if self.mood_name.is_empty() {
                                "Нет настроений"
                            } else {
                                &self.mood_name
                            })
                            .show_ui(ui, |ui| {
                                for (name, entries) in &self.moods {
                                    if !entries.is_empty()
                                        && ui
                                            .selectable_label(self.mood_name == *name, name)
                                            .clicked()
                                    {
                                        self.mood_name = name.clone();
                                        self.mood_index = 0;
                                    }
                                }
                            });
                    });
                    if let Some(centroids) = self.moods.get(&self.mood_name) {
                        egui::ComboBox::from_id_salt("centroid")
                            .selected_text(
                                centroids
                                    .get(self.mood_index)
                                    .map(|m| {
                                        format!("{} ({} треков)", m.top_tags.join(", "), m.n_songs)
                                    })
                                    .unwrap_or_else(|| "Нет вариантов".into()),
                            )
                            .show_ui(ui, |ui| {
                                for (i, mood) in centroids.iter().enumerate() {
                                    ui.selectable_value(
                                        &mut self.mood_index,
                                        i,
                                        format!(
                                            "{} ({} треков)",
                                            mood.top_tags.join(", "),
                                            mood.n_songs
                                        ),
                                    );
                                }
                            });
                    }
                }
                MixMode::Genre => {
                    if ui
                        .add_enabled(!self.busy, egui::Button::new("Загрузить жанры"))
                        .clicked()
                    {
                        let c = self.client.clone().unwrap();
                        self.spawn(ctx, move || Message::Genres(c.genres()));
                    }
                    egui::ComboBox::from_id_salt("genre")
                        .selected_text(if self.genre.is_empty() {
                            "Выберите жанр"
                        } else {
                            &self.genre
                        })
                        .show_ui(ui, |ui| {
                            for genre in &self.genres {
                                ui.selectable_value(&mut self.genre, genre.clone(), genre);
                            }
                        });
                }
                MixMode::Artist => {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.artist)
                            .hint_text("Точное имя исполнителя")
                            .desired_width(300.0),
                    );
                    ui.horizontal(|ui| {
                        ui.label("Разнообразие:");
                        ui.add(egui::Slider::new(&mut self.temperature, 0.0..=5.0));
                    });
                }
            }
            ui.horizontal(|ui| {
                ui.label("Количество:");
                ui.add(egui::Slider::new(&mut self.mix_count, 1..=100));
            });
            if self.mix_mode == MixMode::Track || self.mix_mode == MixMode::Mood {
                ui.checkbox(
                    &mut self.mix_artist_limit,
                    "Ограничить число треков одного исполнителя",
                );
            }
            if self.mix_mode == MixMode::Track {
                ui.checkbox(&mut self.mix_radius, "Плавный переход между треками");
                ui.checkbox(&mut self.mix_mood_filter, "Учитывать настроение трека");
            }
            let valid = match self.mix_mode {
                MixMode::Navidrome | MixMode::Track => self.selected.is_some(),
                MixMode::Wave => true,
                MixMode::Mood => self
                    .moods
                    .get(&self.mood_name)
                    .and_then(|v| v.get(self.mood_index))
                    .is_some(),
                MixMode::Artist => !self.artist.trim().is_empty(),
                MixMode::Genre => !self.genre.is_empty(),
            };
            if ui
                .add_enabled(!self.busy && valid, egui::Button::new("СОЗДАТЬ МИКС  →"))
                .clicked()
            {
                self.open_playlist = None;
                let id = self
                    .selected
                    .as_ref()
                    .map(|s| s.id.clone())
                    .unwrap_or_default();
                let count = self.mix_count;
                if self.mix_mode == MixMode::Navidrome {
                    let c = self.client.clone().unwrap();
                    self.spawn(ctx, move || Message::Songs(c.similar(&id, count)));
                } else if self.mix_mode == MixMode::Wave {
                    let c = self.client.clone().unwrap();
                    self.spawn(ctx, move || Message::Songs(c.favorite_wave(count)));
                } else if self.mix_mode == MixMode::Genre {
                    let c = self.client.clone().unwrap();
                    let genre = self.genre.clone();
                    self.spawn(ctx, move || Message::Songs(c.genre_songs(&genre, count)));
                } else {
                    let muse = self.muse.clone().unwrap();
                    let mode = self.mix_mode;
                    let artist = self.artist.clone();
                    let temperature = self.temperature;
                    let mood = self
                        .moods
                        .get(&self.mood_name)
                        .and_then(|v| v.get(self.mood_index))
                        .map(|m| (self.mood_name.clone(), m.index));
                    let options = SimilarOptions {
                        count,
                        artist_limit: self.mix_artist_limit,
                        radius: self.mix_radius,
                        mood_similarity: self.mix_mood_filter,
                    };
                    self.spawn(ctx, move || {
                        Message::Songs(match mode {
                            MixMode::Track => muse.similar(&id, None, options),
                            MixMode::Mood => muse.similar(
                                "",
                                mood.as_ref().map(|(name, index)| (name.as_str(), *index)),
                                options,
                            ),
                            MixMode::Artist => muse.artist_mix(&artist, count, temperature),
                            MixMode::Navidrome | MixMode::Wave | MixMode::Genre => unreachable!(),
                        })
                    });
                }
            }
            if ui
                .add_enabled(
                    !self.busy
                        && valid
                        && matches!(
                            self.mix_mode,
                            MixMode::Navidrome | MixMode::Track | MixMode::Mood | MixMode::Genre
                        ),
                    egui::Button::new("▶ ЗАПУСТИТЬ ВОЛНУ"),
                )
                .clicked()
            {
                let id = self
                    .selected
                    .as_ref()
                    .map(|s| s.id.clone())
                    .unwrap_or_default();
                let options = SimilarOptions {
                    count: 100,
                    artist_limit: self.mix_artist_limit,
                    radius: self.mix_radius,
                    mood_similarity: self.mix_mood_filter,
                };
                let source = match self.mix_mode {
                    MixMode::Navidrome => WaveSource::Navidrome(id),
                    MixMode::Track => WaveSource::Track(id, options),
                    MixMode::Genre => WaveSource::Genre(self.genre.clone()),
                    MixMode::Mood => WaveSource::Mood(
                        self.mood_name.clone(),
                        self.moods[&self.mood_name][self.mood_index].index,
                        options,
                    ),
                    _ => unreachable!(),
                };
                self.start_wave(source, ctx);
            }
        }
        ui.add_space(15.0);
        self.song_list(ui, ctx);
    }
    pub(super) fn playlists_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, accent: Color32) {
        Self::heading(
            ui,
            "04 / PLAYLISTS",
            "Плейлисты",
            "Открывайте и редактируйте подборки на сервере",
            accent,
        );
        if ui
            .add_enabled(!self.busy, egui::Button::new("Обновить плейлисты"))
            .clicked()
        {
            let c = self.client.clone().unwrap();
            self.spawn(ctx, move || {
                c.invalidate_metadata();
                Message::Playlists(c.playlists())
            });
        }
        for p in self.playlists.clone() {
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !self.busy,
                        egui::Button::new(format!("{}  ·  {} треков", p.name, p.song_count)),
                    )
                    .clicked()
                {
                    let c = self.client.clone().unwrap();
                    let id = p.id.clone();
                    self.spawn(ctx, move || {
                        let songs = c.playlist(&id);
                        Message::PlaylistLoaded(id, songs)
                    });
                }
                if ui
                    .add_enabled(
                        !self.busy && self.selected.is_some(),
                        egui::Button::new("+ выбранный трек"),
                    )
                    .clicked()
                {
                    let c = self.client.clone().unwrap();
                    let id = p.id.clone();
                    let song = self.selected.as_ref().unwrap().id.clone();
                    self.spawn(ctx, move || {
                        let result = c.add_to_playlist(&id, &song).and_then(|()| c.playlist(&id));
                        Message::PlaylistLoaded(id, result)
                    });
                }
            });
        }
        ui.add_space(15.0);
        ui.label("Сохранить текущий список треков:");
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.playlist_name).hint_text("Название плейлиста"),
            );
            if ui
                .add_enabled(
                    !self.busy && !self.songs.is_empty(),
                    egui::Button::new("Сохранить"),
                )
                .clicked()
            {
                let c = self.client.clone().unwrap();
                let name = self.playlist_name.clone();
                let songs = self.songs.clone();
                self.spawn(ctx, move || {
                    Message::Created(c.create_playlist(&name, &songs))
                });
            }
        });
        ui.add_space(15.0);
        if let Some(id) = self.open_playlist.clone() {
            if self.songs.is_empty() {
                ui.label("Плейлист пуст.");
            }
            for (index, song) in self.songs.clone().into_iter().enumerate() {
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!self.busy, egui::Button::new("×"))
                        .on_hover_text("Убрать из плейлиста")
                        .clicked()
                    {
                        let c = self.client.clone().unwrap();
                        let id = id.clone();
                        self.spawn(ctx, move || {
                            Message::PlaylistUpdated(
                                c.remove_from_playlist(&id, index)
                                    .and_then(|()| c.playlist(&id)),
                            )
                        });
                    }
                    let label = format!("{:02}  {} — {}", index + 1, song.artist, song.title);
                    let response = ui.selectable_label(
                        self.selected.as_ref().is_some_and(|s| s.id == song.id),
                        label,
                    );
                    if response.clicked() {
                        self.select(song.clone(), ctx);
                    }
                    if response.double_clicked() {
                        self.resume_position = None;
                        self.end_wave();
                        self.queue = self.songs.clone();
                        self.play_index(index, ctx);
                    }
                });
            }
        } else {
            self.song_list(ui, ctx);
        }
    }
    pub(super) fn queue_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, accent: Color32) {
        Self::heading(
            ui,
            "05 / QUEUE",
            "Очередь",
            "Меняйте порядок и управляйте следующими треками",
            accent,
        );
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !self.busy && !self.queue.is_empty(),
                    egui::Button::new("Сохранить на сервере"),
                )
                .clicked()
            {
                let c = self.client.clone().unwrap();
                let songs = self.queue.clone();
                let current = self
                    .queue_index
                    .and_then(|i| self.queue.get(i))
                    .map(|s| s.id.clone());
                let position = self
                    .resume_position
                    .map(|(_, seconds)| seconds)
                    .unwrap_or(self.position)
                    .max(0.0);
                self.spawn(ctx, move || {
                    Message::QueueSaved(c.save_queue(
                        &songs,
                        current.as_deref(),
                        (position * 1000.0) as u64,
                    ))
                });
            }
            if ui
                .add_enabled(!self.busy, egui::Button::new("Восстановить с сервера"))
                .clicked()
            {
                let c = self.client.clone().unwrap();
                self.spawn(ctx, move || Message::QueueLoaded(c.saved_queue()));
            }
            if ui
                .add_enabled(
                    !self.busy
                        && !self.loading_audio
                        && self.player.is_none()
                        && !self.queue.is_empty(),
                    egui::Button::new("Очистить очередь"),
                )
                .clicked()
            {
                self.end_wave();
                self.prefetcher.clear();
                self.queue.clear();
                self.queue_index = None;
                self.resume_position = None;
                self.status = "Очередь очищена".into();
            }
        });
        if self.queue.is_empty() {
            ui.label("Запустите трек из медиатеки, микса или плейлиста.");
        }
        for (index, song) in self.queue.clone().into_iter().enumerate() {
            ui.horizontal(|ui| {
                let editable = !self.loading_audio;
                if ui
                    .add_enabled(editable && index > 0, egui::Button::new("↑"))
                    .clicked()
                    && queue::move_by(&mut self.queue, &mut self.queue_index, index, -1)
                {
                    if let (Some((_, position)), Some(current)) =
                        (self.resume_position, self.queue_index)
                    {
                        self.resume_position = Some((current, position));
                    }
                    self.schedule_prefetch();
                }
                if ui
                    .add_enabled(
                        editable && index + 1 < self.queue.len(),
                        egui::Button::new("↓"),
                    )
                    .clicked()
                    && queue::move_by(&mut self.queue, &mut self.queue_index, index, 1)
                {
                    if let (Some((_, position)), Some(current)) =
                        (self.resume_position, self.queue_index)
                    {
                        self.resume_position = Some((current, position));
                    }
                    self.schedule_prefetch();
                }
                if ui
                    .add_enabled(
                        editable && self.queue_index != Some(index),
                        egui::Button::new("×"),
                    )
                    .on_hover_text("Убрать из очереди")
                    .clicked()
                    && queue::remove(&mut self.queue, &mut self.queue_index, index)
                {
                    if let (Some((_, position)), Some(current)) =
                        (self.resume_position, self.queue_index)
                    {
                        self.resume_position = Some((current, position));
                    }
                    self.schedule_prefetch();
                }
                let label = format!("{:02}  {} — {}", index + 1, song.artist, song.title);
                if ui
                    .selectable_label(self.queue_index == Some(index), label)
                    .double_clicked()
                {
                    self.play_index(index, ctx);
                }
            });
        }
    }
    pub(super) fn lyrics_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, accent: Color32) {
        Self::heading(
            ui,
            "06 / LYRICS",
            "Текст песни",
            "Тексты и синхронизация от сервера",
            accent,
        );
        ui.label(
            self.selected
                .as_ref()
                .map(|song| format!("{} — {}", song.artist, song.title))
                .unwrap_or_else(|| "Выберите трек в медиатеке или очереди".into()),
        );
        if ui
            .add_enabled(
                !self.busy && self.selected.is_some(),
                egui::Button::new("Загрузить текст"),
            )
            .clicked()
        {
            let c = self.client.clone().unwrap();
            let id = self.selected.as_ref().unwrap().id.clone();
            let revision = self.session_revision;
            self.spawn(ctx, move || {
                let lyrics = c.lyrics(&id);
                Message::Lyrics(revision, id, lyrics)
            });
        }
        if self
            .selected
            .as_ref()
            .is_some_and(|song| self.lyrics_song.as_ref() == Some(&song.id))
            && !self.lyrics.is_empty()
        {
            if self.lyrics.len() > 1 {
                egui::ComboBox::from_id_salt("lyrics_language")
                    .selected_text(
                        self.lyrics
                            .get(self.lyrics_index)
                            .map(|l| {
                                if l.lang.is_empty() {
                                    "Без языка"
                                } else {
                                    l.lang.as_str()
                                }
                            })
                            .unwrap_or("Текст"),
                    )
                    .show_ui(ui, |ui| {
                        for (i, lyrics) in self.lyrics.iter().enumerate() {
                            ui.selectable_value(
                                &mut self.lyrics_index,
                                i,
                                if lyrics.lang.is_empty() {
                                    "Без языка"
                                } else {
                                    &lyrics.lang
                                },
                            );
                        }
                    });
            }
            if let Some(lyrics) = self.lyrics.get(self.lyrics_index).cloned() {
                let same_track = self
                    .queue_index
                    .and_then(|i| self.queue.get(i))
                    .zip(self.selected.as_ref())
                    .is_some_and(|(playing, selected)| playing.id == selected.id)
                    && self.player.is_some();
                let mpv = same_track && self.player.as_ref().is_some_and(|p| p.is_mpv);
                let active = if mpv {
                    lyrics.active_line((self.position.max(0.0) * 1000.0) as u64)
                } else {
                    None
                };
                if lyrics.synced && mpv {
                    ui.label(
                        RichText::new("Нажмите на строку, чтобы перейти к ней")
                            .size(11.0)
                            .color(Color32::GRAY),
                    );
                }
                ui.add_space(10.0);
                for (index, line) in lyrics.lines.iter().enumerate() {
                    let response = ui.selectable_label(active == Some(index), &line.value);
                    if mpv && response.clicked() {
                        if let Some(position) = lyrics.seek_position(index) {
                            self.last_listened_position = None;
                            self.player_command(
                                ctx,
                                PlayerAction::Seek,
                                serde_json::json!(["seek", position, "absolute+exact"]),
                            );
                        }
                    }
                }
            }
        }
    }
    pub(super) fn settings_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, accent: Color32) {
        Self::heading(
            ui,
            "05 / SETTINGS",
            "Настройки",
            "Цвета и подключение AudioMuse-AI",
            accent,
        );
        ui.label("Цветовая палитра");
        ui.horizontal(|ui| {
            for (i, name) in ["Мята", "Лёд", "Сирень", "Янтарь"].iter().enumerate()
            {
                if ui.selectable_label(self.palette == i, *name).clicked() {
                    self.palette = i;
                }
            }
        });
        ui.add_space(20.0);
        ui.heading("AudioMuse-AI");
        if self.muse.is_some() {
            ui.label("Подключён");
            if ui
                .add_enabled(!self.busy, egui::Button::new("Отключить AudioMuse-AI"))
                .clicked()
            {
                self.muse = None;
                self.moods.clear();
                self.status = "AudioMuse-AI отключён".into();
            }
        } else {
            ui.label("Адрес AudioMuse-AI");
            ui.add(egui::TextEdit::singleline(&mut self.muse_url).desired_width(400.0));
            ui.label("Сервер внутри AudioMuse-AI (необязательно)");
            ui.add(
                egui::TextEdit::singleline(&mut self.muse_server)
                    .hint_text("По умолчанию")
                    .desired_width(400.0),
            );
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.auth_mode, AuthMode::Login, "Логин и пароль");
                ui.selectable_value(&mut self.auth_mode, AuthMode::Token, "API-токен");
                ui.selectable_value(&mut self.auth_mode, AuthMode::None, "Без авторизации");
            });
            match self.auth_mode {
                AuthMode::Login => {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.muse_user)
                            .hint_text("Логин AudioMuse-AI"),
                    );
                    ui.add(
                        egui::TextEdit::singleline(&mut self.muse_password)
                            .password(true)
                            .hint_text("Пароль AudioMuse-AI"),
                    );
                }
                AuthMode::Token => {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.muse_token)
                            .password(true)
                            .hint_text("API-токен"),
                    );
                    ui.label(
                        RichText::new(
                            "API-токен предоставляет административный доступ к AudioMuse-AI.",
                        )
                        .size(11.0)
                        .color(Color32::GRAY),
                    );
                }
                AuthMode::None => {
                    ui.label("Для сервера с отключённой авторизацией.");
                }
            }
            if ui
                .add_enabled(!self.busy, egui::Button::new("Подключить AudioMuse-AI"))
                .clicked()
            {
                let url = self.muse_url.clone();
                let server = self.muse_server.clone();
                let auth = match self.auth_mode {
                    AuthMode::None => Authentication::None,
                    AuthMode::Token => Authentication::Token(self.muse_token.clone()),
                    AuthMode::Login => Authentication::Login {
                        username: self.muse_user.clone(),
                        password: self.muse_password.clone(),
                    },
                };
                self.spawn(ctx, move || {
                    Message::MuseConnected(AudioMuse::connect(&url, auth, &server))
                });
            }
        }
        ui.add_space(20.0);
        ui.heading("Звук");
        ui.label("ReplayGain для mpv (применяется к следующему треку)");
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.replay_gain, player::ReplayGain::Off, "Выкл");
            ui.selectable_value(&mut self.replay_gain, player::ReplayGain::Track, "Трек");
            ui.selectable_value(&mut self.replay_gain, player::ReplayGain::Album, "Альбом");
        });
        ui.label(
            RichText::new("Нужны теги ReplayGain в аудиофайле; ffplay не поддерживается.")
                .size(11.0)
                .color(Color32::GRAY),
        );
        ui.add_space(20.0);
        ui.heading("Таймер сна");
        ui.horizontal_wrapped(|ui| {
            for minutes in [15, 30, 60] {
                if ui.button(format!("{minutes} мин")).clicked() {
                    self.sleep_timer =
                        SleepTimer::after(Instant::now(), Duration::from_secs(minutes * 60));
                }
            }
            if ui.button("До конца трека").clicked() {
                self.sleep_timer = SleepTimer::EndOfTrack;
            }
            if ui.button("Выключить").clicked() {
                self.sleep_timer = SleepTimer::Off;
            }
        });
        match self.sleep_timer {
            SleepTimer::Off => {
                ui.label("Таймер выключен");
            }
            SleepTimer::EndOfTrack => {
                ui.label("Остановить после текущего трека");
            }
            timer => {
                let seconds = timer
                    .remaining(Instant::now())
                    .unwrap_or_default()
                    .as_secs();
                ui.label(format!("Осталось {:02}:{:02}", seconds / 60, seconds % 60));
            }
        }
        ui.add_space(20.0);
        ui.heading("Кэш музыки");
        if ui
            .checkbox(
                &mut self.prefetch_enabled,
                "Загружать следующие два трека заранее",
            )
            .changed()
        {
            self.schedule_prefetch();
        }
        ui.horizontal(|ui| {
            ui.label("Максимум аудио, ГиБ:");
            let slider = ui.add_enabled(!self.busy, egui::Slider::new(&mut self.cache_gib, 1..=20));
            if slider.drag_stopped() || (slider.changed() && !slider.dragged()) {
                let c = self.client.clone().unwrap();
                let bytes = self.cache_gib as u64 * 1024 * 1024 * 1024;
                self.spawn(ctx, move || {
                    Message::CacheChanged(c.set_audio_cache_limit(bytes))
                });
            }
        });
        if let Some(info) = self.cache_info {
            ui.label(format!(
                "Треки: {} · {:.1} МиБ; обложки и данные: {:.1} МиБ{}",
                info.audio_files,
                info.audio_bytes as f64 / 1_048_576.0,
                (info.cover_bytes + info.metadata_bytes) as f64 / 1_048_576.0,
                if info.audio_over_limit {
                    " · лимит превышен"
                } else {
                    ""
                }
            ));
        }
        if ui
            .add_enabled(!self.busy, egui::Button::new("Проверить размер кэша"))
            .clicked()
        {
            let c = self.client.clone().unwrap();
            self.spawn(ctx, move || Message::CacheInfo(c.cache_stats()));
        }
        if ui
            .add_enabled(
                !self.busy && self.player.is_none() && !self.loading_audio,
                egui::Button::new("Очистить кэш этого аккаунта"),
            )
            .clicked()
        {
            self.prefetcher.clear();
            let c = self.client.clone().unwrap();
            self.spawn(ctx, move || {
                let mut result = c.clear_cache();
                for _ in 0..20 {
                    if result.is_ok() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                    result = c.clear_cache();
                }
                Message::CacheChanged(result)
            });
        }
        ui.add_space(25.0);
        if ui
            .add_enabled(!self.busy, egui::Button::new("Отключиться от Navidrome"))
            .clicked()
        {
            self.end_wave();
            self.prefetcher.clear();
            self.stop();
            self.client = None;
            self.session_revision = self.session_revision.wrapping_add(1);
            self.muse = None;
            self.moods.clear();
            self.songs.clear();
            self.albums.clear();
            self.artists.clear();
            self.genres.clear();
            self.genre.clear();
            self.cache_info = None;
            self.artist_filter.clear();
            self.album_artist = None;
            self.album_offset = 0;
            self.sleep_timer = SleepTimer::Off;
            self.library_view = LibraryView::Songs;
            self.playlists.clear();
            self.queue.clear();
            self.queue_index = None;
            self.resume_position = None;
            self.open_playlist = None;
            self.selected = None;
            self.cover = None;
            self.lyrics.clear();
            self.lyrics_song = None;
            self.status = "Соединение закрыто".into();
        }
    }
}
