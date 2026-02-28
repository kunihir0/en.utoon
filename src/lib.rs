#![no_std]
use aidoku::{
    prelude::*,
    alloc::{String, Vec, string::ToString, format},
    imports::{
        html::{Element, Document},
        net::{Request, TimeUnit, set_rate_limit},
    },
    Chapter, DeepLinkHandler, DeepLinkResult, FilterValue, Home, HomeComponent, HomeComponentValue,
    HomeLayout, Listing, ListingProvider, Manga, MangaPageResult, MangaStatus,
    Page, PageContent, Result, Source, Viewer,
};

const BASE_URL: &str = "https://utoon.net";

struct UtoonSource;

fn parse_manga_list(html: Document) -> Result<MangaPageResult> {
    let mut entries = Vec::new();
    let has_next_page = html.select_first(".next, .nextpostslink").is_some();

    for item in html.select(".page-item-detail, .c-tabs-item__content").into_iter().flatten() {
        let a = item.select_first(".post-title a");
        if let Some(a_node) = a {
            let url = a_node.attr("href").unwrap_or_default();
            let title = a_node.text().unwrap_or_default();
            if title.is_empty() {
                continue;
            }
            
            let mut cover = "".to_string();
            if let Some(img) = item.select_first("img") {
                cover = img.attr("data-src").unwrap_or_default();
                if cover.is_empty() {
                    cover = img.attr("src").unwrap_or_default();
                }
            }

            if !url.is_empty() {
                let key = url.replace(BASE_URL, "").replace("/manga/", "").replace("/", "");
                entries.push(Manga {
                    key,
                    cover: Some(cover),
                    title,
                    url: Some(url),
                    ..Default::default()
                });
            }
        }
    }

    Ok(MangaPageResult { entries, has_next_page })
}

impl Source for UtoonSource {
    fn new() -> Self {
        set_rate_limit(2, 1, TimeUnit::Seconds);
        Self
    }

    fn get_search_manga_list(
        &self,
        query: Option<String>,
        page: i32,
        filters: Vec<FilterValue>,
    ) -> Result<MangaPageResult> {
        let mut q = String::new();
        let mut sort_by = String::from("latest");

        if let Some(query_str) = query {
            q = query_str.replace(" ", "+");
        }

        for filter in filters {
            match filter {
                FilterValue::Sort { id, index, .. } => {
                    if id == "sort" {
                        sort_by = match index {
                            0 => String::from("latest"),
                            1 => String::from("alphabet"),
                            2 => String::from("rating"),
                            3 => String::from("trending"),
                            4 => String::from("views"),
                            5 => String::from("new-manga"),
                            _ => String::from("latest"),
                        };
                    }
                }
                _ => continue,
            }
        }

        let url = if q.is_empty() {
            format!("{}/manga/page/{}/?m_orderby={}", BASE_URL, page, sort_by)
        } else {
            format!("{}/page/{}/?s={}&post_type=wp-manga", BASE_URL, page, q)
        };

        let html = Request::get(&url)?.html()?;
        parse_manga_list(html)
    }

    fn get_manga_update(
        &self,
        mut manga: Manga,
        needs_details: bool,
        needs_chapters: bool,
    ) -> Result<Manga> {
        let url = format!("{}/manga/{}/", BASE_URL, manga.key);
        
        if needs_details {
            let html = Request::get(&url)?.html()?;
            manga.title = html.select_first(".post-title h1").and_then(|e: Element| e.text()).unwrap_or(manga.title);
            
            if let Some(img) = html.select_first(".summary_image img") {
                let mut cover = img.attr("data-src").unwrap_or_default();
                if cover.is_empty() {
                    cover = img.attr("src").unwrap_or_default();
                }
                if !cover.is_empty() {
                    manga.cover = Some(cover);
                }
            }
            
            manga.description = html.select_first(".summary__content p").and_then(|e: Element| e.text());
            manga.url = Some(url.clone());
            manga.viewer = Viewer::Webtoon;
            
            let mut authors = Vec::new();
            for item in html.select(".author-content a").into_iter().flatten() {
                if let Some(a) = item.text() {
                    authors.push(a);
                }
            }
            if !authors.is_empty() {
                manga.authors = Some(authors);
            }
            
            let status_str = html.select_first(".post-status .summary-content").and_then(|e: Element| e.text()).unwrap_or_default().to_lowercase();
            manga.status = if status_str.contains("ongoing") {
                MangaStatus::Ongoing
            } else if status_str.contains("completed") {
                MangaStatus::Completed
            } else if status_str.contains("canceled") || status_str.contains("dropped") {
                MangaStatus::Cancelled
            } else if status_str.contains("hiatus") {
                MangaStatus::Hiatus
            } else {
                MangaStatus::Unknown
            };

            let mut tags = Vec::new();
            for item in html.select(".genres-content a").into_iter().flatten() {
                if let Some(g) = item.text() {
                    tags.push(g);
                }
            }
            if !tags.is_empty() {
                manga.tags = Some(tags);
            }
        }

        if needs_chapters {
            let mut chapters = Vec::new();
            let mut html = Request::get(&url)?.html()?;
            
            let mut chapter_nodes: Vec<Element> = html.select(".wp-manga-chapter").into_iter().flatten().collect();
            
            if chapter_nodes.is_empty() {
                let ajax_url = format!("{}/wp-admin/admin-ajax.php", BASE_URL);
                let manga_id_post = html.select_first(".rating-post-id").and_then(|e: Element| e.attr("value")).unwrap_or_default();
                
                if !manga_id_post.is_empty() {
                    let body = format!("action=manga_get_chapters&manga={}", manga_id_post);
                    if let Ok(req) = Request::post(&ajax_url) {
                        if let Ok(ajax_html) = req.header("Content-Type", "application/x-www-form-urlencoded")
                            .body(body)
                            .html() 
                        {
                            let nodes: Vec<Element> = ajax_html.select(".wp-manga-chapter").into_iter().flatten().collect();
                            chapter_nodes = nodes;
                        }
                    }
                }
            }

            let mut num = chapter_nodes.len() as f32;
            
            for item in chapter_nodes {
                if let Some(a) = item.select_first("a") {
                    let chapter_url = a.attr("href").unwrap_or_default();
                    if chapter_url.is_empty() {
                        continue;
                    }
                    
                    let title = a.text();
                    let locked = item.attr("class").unwrap_or_default().contains("premium");
                    let key = chapter_url.replace(BASE_URL, "").trim_matches('/').to_string();
                    
                    let mut chapter_number = num;
                    if let Some(ref t) = title {
                        let lower = t.to_lowercase();
                        // Find "chapter " or "ch." or "ch " 
                        let parts: Vec<&str> = lower.split_whitespace().collect();
                        let mut found = false;
                        for i in 0..parts.len() {
                            if parts[i] == "chapter" || parts[i] == "ch" || parts[i] == "ch." || parts[i] == "chap" || parts[i] == "chap." {
                                if i + 1 < parts.len() {
                                    let text_num = parts[i + 1].chars().take_while(|c| c.is_digit(10) || *c == '.').collect::<String>();
                                    if let Ok(n) = text_num.parse::<f32>() {
                                        chapter_number = n;
                                        found = true;
                                        break;
                                    }
                                }
                            }
                        }
                        if !found {
                            for part in parts {
                                let text_num = part.chars().take_while(|c| c.is_digit(10) || *c == '.').collect::<String>();
                                if !text_num.is_empty() {
                                    if let Ok(n) = text_num.parse::<f32>() {
                                        chapter_number = n;
                                        break;
                                    }
                                }
                            }
                        }
                    }

                    chapters.push(Chapter {
                        key,
                        title,
                        url: Some(chapter_url),
                        chapter_number: Some(chapter_number),
                        locked,
                        ..Default::default()
                    });
                    num -= 1.0;
                }
            }
            manga.chapters = Some(chapters);
        }

        Ok(manga)
    }

    fn get_page_list(&self, _manga: Manga, chapter: Chapter) -> Result<Vec<Page>> {
        let url = chapter.url.unwrap_or_else(|| format!("{}/{}/", BASE_URL, chapter.key));
        let html = Request::get(&url)?.html()?;
        
        let mut pages = Vec::new();
        
        for item in html.select(".reading-content img, .page-break img.wp-manga-chapter-img").into_iter().flatten() {
            let mut img_url = item.attr("data-src").unwrap_or_default();
            if img_url.is_empty() {
                img_url = item.attr("data-lazy-src").unwrap_or_default();
            }
            if img_url.is_empty() {
                img_url = item.attr("src").unwrap_or_default();
            }
            
            let img_url = img_url.trim().to_string();
            if !img_url.is_empty() {
                pages.push(Page {
                    content: PageContent::url(img_url),
                    ..Default::default()
                });
            }
        }
        
        if pages.is_empty() {
            // Alternative selector often used in Madara configs
            for item in html.select("img[id^='image-']").into_iter().flatten() {
                let mut img_url = item.attr("data-src").unwrap_or_default();
                if img_url.is_empty() {
                    img_url = item.attr("src").unwrap_or_default();
                }
                
                let img_url = img_url.trim().to_string();
                if !img_url.is_empty() {
                    pages.push(Page {
                        content: PageContent::url(img_url),
                        ..Default::default()
                    });
                }
            }
        }
        
        Ok(pages)
    }
}

impl ListingProvider for UtoonSource {
    fn get_manga_list(&self, listing: Listing, page: i32) -> Result<MangaPageResult> {
        let sort_by = match listing.name.as_str() {
            "Latest Updates" => "latest",
            "Trending" => "trending",
            "Top Rated" => "rating",
            "Most Views" => "views",
            "New" | "New Releases" => "new-manga",
            _ => "latest",
        };
        let url = format!("{}/manga/page/{}/?m_orderby={}", BASE_URL, page, sort_by);
        let html = Request::get(&url)?.html()?;
        parse_manga_list(html)
    }
}

impl Home for UtoonSource {
    fn get_home(&self) -> Result<HomeLayout> {
        let mut components = Vec::new();
        
        // Madara theme doesn't always have simple structured lists on the homepage.
        // It's often easier to define fixed listings for Home layouts based on orderby.
        
        let url_trending = format!("{}/manga/?m_orderby=trending", BASE_URL);
        if let Ok(html) = Request::get(&url_trending).unwrap().html() {
            if let Ok(result) = parse_manga_list(html) {
                if !result.entries.is_empty() {
                    components.push(HomeComponent {
                        title: Some("Trending".to_string()),
                        subtitle: None,
                        value: HomeComponentValue::Scroller {
                            entries: result.entries.into_iter().map(Into::into).collect(),
                            listing: Some(Listing {
                                id: "trending".to_string(),
                                name: "Trending".to_string(),
                                ..Default::default()
                            }),
                        },
                    });
                }
            }
        }

        let url_top_rated = format!("{}/manga/?m_orderby=rating", BASE_URL);
        if let Ok(html) = Request::get(&url_top_rated).unwrap().html() {
            if let Ok(result) = parse_manga_list(html) {
                if !result.entries.is_empty() {
                    components.push(HomeComponent {
                        title: Some("Top Rated".to_string()),
                        subtitle: None,
                        value: HomeComponentValue::Scroller {
                            entries: result.entries.into_iter().map(Into::into).collect(),
                            listing: Some(Listing {
                                id: "rating".to_string(),
                                name: "Top Rated".to_string(),
                                ..Default::default()
                            }),
                        },
                    });
                }
            }
        }

        let url_most_views = format!("{}/manga/?m_orderby=views", BASE_URL);
        if let Ok(html) = Request::get(&url_most_views).unwrap().html() {
            if let Ok(result) = parse_manga_list(html) {
                if !result.entries.is_empty() {
                    components.push(HomeComponent {
                        title: Some("Most Views".to_string()),
                        subtitle: None,
                        value: HomeComponentValue::Scroller {
                            entries: result.entries.into_iter().map(Into::into).collect(),
                            listing: Some(Listing {
                                id: "views".to_string(),
                                name: "Most Views".to_string(),
                                ..Default::default()
                            }),
                        },
                    });
                }
            }
        }

        let url_new = format!("{}/manga/?m_orderby=new-manga", BASE_URL);
        if let Ok(html) = Request::get(&url_new).unwrap().html() {
            if let Ok(result) = parse_manga_list(html) {
                if !result.entries.is_empty() {
                    components.push(HomeComponent {
                        title: Some("New".to_string()),
                        subtitle: None,
                        value: HomeComponentValue::Scroller {
                            entries: result.entries.into_iter().map(Into::into).collect(),
                            listing: Some(Listing {
                                id: "new-manga".to_string(),
                                name: "New".to_string(),
                                ..Default::default()
                            }),
                        },
                    });
                }
            }
        }

        let url_latest = format!("{}/manga/?m_orderby=latest", BASE_URL);
        if let Ok(html) = Request::get(&url_latest).unwrap().html() {
            if let Ok(result) = parse_manga_list(html) {
                if !result.entries.is_empty() {
                    components.push(HomeComponent {
                        title: Some("Latest Updates".to_string()),
                        subtitle: None,
                        value: HomeComponentValue::MangaList {
                            ranking: false,
                            page_size: Some(20),
                            entries: result.entries.into_iter().map(Into::into).collect(),
                            listing: Some(Listing {
                                id: "latest".to_string(),
                                name: "Latest Updates".to_string(),
                                ..Default::default()
                            }),
                        },
                    });
                }
            }
        }

        Ok(HomeLayout { components })
    }
}

impl DeepLinkHandler for UtoonSource {
    fn handle_deep_link(&self, url: String) -> Result<Option<DeepLinkResult>> {
        if url.contains("/manga/") {
            let parts: Vec<&str> = url.split("/manga/").collect();
            if parts.len() > 1 {
                let id_parts: Vec<&str> = parts[1].split('/').collect();
                if !id_parts.is_empty() && !id_parts[0].is_empty() {
                    return Ok(Some(DeepLinkResult::Manga {
                        key: id_parts[0].to_string(),
                    }));
                }
            }
        }
        Ok(None)
    }
}

register_source!(UtoonSource, ListingProvider, Home, DeepLinkHandler);


