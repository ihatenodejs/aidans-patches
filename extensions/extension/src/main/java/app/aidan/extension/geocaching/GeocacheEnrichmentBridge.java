package app.aidan.extension.geocaching;

import android.database.Cursor;
import android.util.Log;
import java.io.BufferedReader;
import java.io.InputStream;
import java.io.InputStreamReader;
import java.lang.reflect.Constructor;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.net.HttpURLConnection;
import java.net.URL;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Collections;
import java.util.Date;
import java.util.List;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

public final class GeocacheEnrichmentBridge {
    private static final String TAG = "GeocacheEnrichment";

    private static final Map<String, Object> sRememberedLiteCaches = new ConcurrentHashMap<>();
    private static final Map<String, Object> sEnrichedCaches = new ConcurrentHashMap<>();

    private static final Pattern PATTERN_OWNER =
            Pattern.compile("A cache by <a[^>]*>(.*?)</a>", Pattern.CASE_INSENSITIVE);
    private static final Pattern PATTERN_SHORT_DESC =
            Pattern.compile("id=[\"']ctl00_ContentBody_ShortDescription[\"'][^>]*>(.*?)</span>",
                    Pattern.CASE_INSENSITIVE | Pattern.DOTALL);
    private static final Pattern PATTERN_LONG_DESC =
            Pattern.compile("id=[\"']ctl00_ContentBody_LongDescription[\"'][^>]*>(.*?)</span>",
                    Pattern.CASE_INSENSITIVE | Pattern.DOTALL);
    private static final Pattern PATTERN_HINT =
            Pattern.compile("id=[\"']div_hint[\"'][^>]*>(.*?)</div>",
                    Pattern.CASE_INSENSITIVE | Pattern.DOTALL);
    private static final Pattern PATTERN_NAME =
            Pattern.compile("id=[\"']ctl00_ContentBody_CacheName[\"'][^>]*>(.*?)</span>",
                    Pattern.CASE_INSENSITIVE);

    private GeocacheEnrichmentBridge() {
    }

    /**
     * Called when a LiteGeocache is converted to a LegacyGeocache in LegacyGeocacheRepo.loadGeocache.
     * Caches the in-memory representation so that fallback enrichment can use its exact coordinates,
     * difficulty, terrain, attributes, and size without re-querying SQLite.
     */
    public static void rememberLiteGeocache(Object legacyGeocache) {
        if (legacyGeocache == null) {
            return;
        }
        try {
            Field codeField = legacyGeocache.getClass().getField("code");
            String code = (String) codeField.get(legacyGeocache);
            if (code != null && !code.isEmpty()) {
                sRememberedLiteCaches.put(code.toUpperCase(), legacyGeocache);
            }
        } catch (Throwable t) {
            Log.d(TAG, "rememberLiteGeocache error: " + t);
        }
    }

    /**
     * Intercepts fetchLegacyCacheFromServer failure in LegacyGeocacheRepo$fetchLegacyCacheFromServer$2.
     * When basic accounts get HTTP 403 Forbidden for traditional caches with D > 1.5 / T > 1.5 (or
     * non-traditional caches), scrapes the public webpage (where non-PMO cache descriptions and hints
     * are freely accessible) and constructs an enriched LegacyGeocache wrapped in Success (xk9).
     */
    public static Object onFetchFailed(Object suspendLambda, Object failure) {
        if (suspendLambda == null) {
            return failure;
        }
        try {
            if (!isForbiddenFailure(failure)) {
                return failure;
            }

            Class<?> lambdaClass = suspendLambda.getClass();
            Field nRepoField = lambdaClass.getField("N");
            Field mCodeField = lambdaClass.getField("M");
            Object repo = nRepoField.get(suspendLambda);
            String referenceCode = (String) mCodeField.get(suspendLambda);

            if (referenceCode == null || referenceCode.isEmpty()) {
                return failure;
            }
            String upperCode = referenceCode.toUpperCase();

            // Extract any logs or waypoints that succeeded in the parallel coroutine calls
            Object logsObj = getFieldOrNull(lambdaClass, suspendLambda, "H");
            Object addWptObj = getFieldOrNull(lambdaClass, suspendLambda, "I");
            Object userWptObj = getFieldOrNull(lambdaClass, suspendLambda, "J");

            Object enriched = enrichGeocache(repo, upperCode, logsObj, addWptObj, userWptObj);
            if (enriched != null) {
                Class<?> xk9Class = Class.forName("xk9");
                Constructor<?> ctor = xk9Class.getDeclaredConstructor(Object.class);
                return ctor.newInstance(enriched);
            }
        } catch (Throwable t) {
            Log.e(TAG, "onFetchFailed enrichment failed", t);
        }
        return failure;
    }

    /**
     * Overload for direct repo + referenceCode call.
     */
    public static Object onFetchFailed(Object repo, String referenceCode, Object failure) {
        if (referenceCode == null || referenceCode.isEmpty() || repo == null) {
            return failure;
        }
        try {
            if (!isForbiddenFailure(failure)) {
                return failure;
            }
            String upperCode = referenceCode.toUpperCase();
            Object enriched = enrichGeocache(repo, upperCode, null, null, null);
            if (enriched != null) {
                Class<?> xk9Class = Class.forName("xk9");
                Constructor<?> ctor = xk9Class.getDeclaredConstructor(Object.class);
                return ctor.newInstance(enriched);
            }
        } catch (Throwable t) {
            Log.e(TAG, "onFetchFailed enrichment failed for " + referenceCode, t);
        }
        return failure;
    }

    private static Object getFieldOrNull(Class<?> clazz, Object target, String name) {
        try {
            Field f = clazz.getField(name);
            return f.get(target);
        } catch (Throwable ignored) {
            return null;
        }
    }

    private static boolean isForbiddenFailure(Object failure) {
        if (failure == null) {
            return false;
        }
        try {
            Class<?> clazz = failure.getClass();
            Field aField = clazz.getField("a");
            Object err = aField.get(failure);
            if (err != null) {
                String errName = err.getClass().getName();
                if (errName.endsWith("$e$e") || errName.endsWith("$e$f") || err.toString().contains("Forbidden")) {
                    return true;
                }
            }
        } catch (Throwable t) {
            String s = failure.toString();
            return s != null && s.contains("Forbidden");
        }
        return false;
    }

    public static Object enrichGeocache(Object repo, String code, Object logsResult, Object addWptResult, Object userWptResult) {
        Object cached = sEnrichedCaches.get(code);
        if (cached != null) {
            return cached;
        }

        // 1. Look for base LegacyGeocache (from memory or DB)
        Object baseCache = sRememberedLiteCaches.get(code);
        if (baseCache == null && repo != null) {
            try {
                Field aField = repo.getClass().getField("a"); // Ly82;
                Object y82 = aField.get(repo);
                if (y82 != null) {
                    Method pMethod = y82.getClass().getMethod("p", String.class);
                    baseCache = pMethod.invoke(y82, code);
                }
            } catch (Throwable ignored) {
            }
        }

        if (baseCache == null && repo != null) {
            baseCache = queryLiteGeocacheFromDb(repo, code);
        }

        // 2. Fetch and scrape public cache page
        ScrapedData scraped = scrapePublicCachePage(code);
        if (scraped == null) {
            Log.w(TAG, "Public page could not be scraped for " + code);
            return null;
        }

        try {
            Object enriched;
            if (baseCache != null) {
                enriched = createEnrichedCopy(baseCache, scraped, logsResult, addWptResult, userWptResult);
            } else {
                enriched = createLegacyGeocacheFromScraped(scraped);
            }

            if (enriched != null) {
                // Save to y82 SQLite database (Geocache table with IsLite = 0)
                if (repo != null) {
                    try {
                        Field aField = repo.getClass().getField("a");
                        Object y82 = aField.get(repo);
                        if (y82 != null) {
                            Method jMethod = y82.getClass().getMethod("J", List.class);
                            jMethod.invoke(y82, Collections.singletonList(enriched));
                        }
                    } catch (Throwable t) {
                        Log.e(TAG, "Failed to persist enriched cache into y82", t);
                    }
                }

                sEnrichedCaches.put(code, enriched);
                return enriched;
            }
        } catch (Throwable t) {
            Log.e(TAG, "Failed to build enriched LegacyGeocache for " + code, t);
        }
        return null;
    }

    private static Object createEnrichedCopy(Object origCache, ScrapedData scraped, Object logsResult, Object addWptResult, Object userWptResult) throws Exception {
        Class<?> builderClass = Class.forName("com.groundspeak.geocaching.intro.types.LegacyGeocache$Builder");
        Object builder = builderClass.getDeclaredConstructor().newInstance();
        Class<?> cacheClass = origCache.getClass();

        copyField(cacheClass, origCache, builderClass, builder, "code", "c");
        copyField(cacheClass, origCache, builderClass, builder, "name", "x");
        copyField(cacheClass, origCache, builderClass, builder, "cacheType", "b");
        copyField(cacheClass, origCache, builderClass, builder, "containerType", "d");
        copyField(cacheClass, origCache, builderClass, builder, "dateCreated", "e");
        copyField(cacheClass, origCache, builderClass, builder, "dateLastUpdate", "f");
        copyField(cacheClass, origCache, builderClass, builder, "dateLastVisited", "g");
        copyField(cacheClass, origCache, builderClass, builder, "difficulty", "h");
        copyField(cacheClass, origCache, builderClass, builder, "favoritePoints", "j");
        copyField(cacheClass, origCache, builderClass, builder, "guid", "k");
        copyField(cacheClass, origCache, builderClass, builder, "hasBeenFavoritedByUser", "l");
        copyField(cacheClass, origCache, builderClass, builder, "isUnlockedForUser", "m");
        copyField(cacheClass, origCache, builderClass, builder, "id", "n");
        copyField(cacheClass, origCache, builderClass, builder, "isLocked", "o");
        copyField(cacheClass, origCache, builderClass, builder, "isPremium", "p");
        copyField(cacheClass, origCache, builderClass, builder, "isPublished", "q");
        copyField(cacheClass, origCache, builderClass, builder, "available", "r");
        copyField(cacheClass, origCache, builderClass, builder, "isHighlyFavorited", "s");
        copyField(cacheClass, origCache, builderClass, builder, "latitude", "t");
        copyField(cacheClass, origCache, builderClass, builder, "longitude", "w");
        copyField(cacheClass, origCache, builderClass, builder, "publishDateUtc", "z");
        copyField(cacheClass, origCache, builderClass, builder, "terrain", "C");
        copyField(cacheClass, origCache, builderClass, builder, "trackableCount", "D");
        copyField(cacheClass, origCache, builderClass, builder, "eventStartDateUtc", "E");
        copyField(cacheClass, origCache, builderClass, builder, "eventEndDateUtc", "F");
        copyField(cacheClass, origCache, builderClass, builder, "cacheTimeZone", "G");
        copyField(cacheClass, origCache, builderClass, builder, "utcPlaceDate", "H");
        copyField(cacheClass, origCache, builderClass, builder, "owner", "I");
        copyField(cacheClass, origCache, builderClass, builder, "lastModified", "J");
        copyField(cacheClass, origCache, builderClass, builder, "geoTourInfo", "R");
        copyField(cacheClass, origCache, builderClass, builder, "draftGuid", "L");
        copyField(cacheClass, origCache, builderClass, builder, "foundDate", "S");
        copyField(cacheClass, origCache, builderClass, builder, "dnfDate", "T");
        copyField(cacheClass, origCache, builderClass, builder, "willAttendDate", "U");
        copyField(cacheClass, origCache, builderClass, builder, "attributes", "Q");
        copyField(cacheClass, origCache, builderClass, builder, "a", "V");
        copyField(cacheClass, origCache, builderClass, builder, "unpublishedState", "W");
        copyField(cacheClass, origCache, builderClass, builder, "hasUncollectedShareable", "X");

        // Copy collections
        copyListField(cacheClass, origCache, builderClass, builder, "geocacheLogs", "M");
        copyListField(cacheClass, origCache, builderClass, builder, "additionalWaypoints", "N");
        copyListField(cacheClass, origCache, builderClass, builder, "userWaypoints", "O");
        copyListField(cacheClass, origCache, builderClass, builder, "images", "P");

        // If parallel coroutine logs or waypoints succeeded, add them
        extractAndAppendResultList(logsResult, builderClass, builder, "M");
        extractAndAppendResultList(addWptResult, builderClass, builder, "N");
        extractAndAppendResultList(userWptResult, builderClass, builder, "O");

        // Overwrite enriched fields from webpage
        builderClass.getField("K").set(builder, false); // isLite = false
        String placedBy = (scraped.placedBy != null && !scraped.placedBy.isEmpty())
                ? scraped.placedBy
                : (String) cacheClass.getField("placedBy").get(origCache);
        builderClass.getField("y").set(builder, placedBy != null ? placedBy : "");
        builderClass.getField("A").set(builder, scraped.shortDescription != null ? scraped.shortDescription : "");
        builderClass.getField("u").set(builder, scraped.longDescription != null ? scraped.longDescription : "");
        builderClass.getField("i").set(builder, scraped.encodedHints != null ? scraped.encodedHints : "");
        builderClass.getField("v").set(builder, Boolean.TRUE); // longDescriptionIsHtml
        builderClass.getField("B").set(builder, Boolean.TRUE); // shortDescriptionIsHtml

        return cacheClass.getDeclaredConstructor(builderClass).newInstance(builder);
    }

    private static void copyField(Class<?> srcClass, Object src, Class<?> dstClass, Object dst, String srcField, String dstField) {
        try {
            Field s = srcClass.getField(srcField);
            Field d = dstClass.getField(dstField);
            d.set(dst, s.get(src));
        } catch (Throwable ignored) {
        }
    }

    @SuppressWarnings("unchecked")
    private static void copyListField(Class<?> srcClass, Object src, Class<?> dstClass, Object dst, String srcField, String dstField) {
        try {
            Field s = srcClass.getField(srcField);
            Field d = dstClass.getField(dstField);
            List srcList = (List) s.get(src);
            List dstList = (List) d.get(dst);
            if (srcList != null && dstList != null && dstList.isEmpty()) {
                dstList.addAll(srcList);
            }
        } catch (Throwable ignored) {
        }
    }

    @SuppressWarnings("unchecked")
    private static void extractAndAppendResultList(Object resultWrapper, Class<?> builderClass, Object builder, String dstField) {
        if (resultWrapper == null) return;
        try {
            Class<?> xk9Class = Class.forName("xk9");
            if (xk9Class.isInstance(resultWrapper)) {
                Field aField = xk9Class.getField("a");
                Object content = aField.get(resultWrapper);
                List listToAdd = null;
                if (content instanceof List) {
                    listToAdd = (List) content;
                } else if (content != null) {
                    // CacheUserWaypointsResponse has .a (List)
                    // AdditionalWaypointsForCache has .a (List)
                    Field subA = content.getClass().getField("a");
                    Object subObj = subA.get(content);
                    if (subObj instanceof List) {
                        listToAdd = (List) subObj;
                    }
                }
                if (listToAdd != null && !listToAdd.isEmpty()) {
                    Field dField = builderClass.getField(dstField);
                    List dstList = (List) dField.get(builder);
                    if (dstList != null) {
                        dstList.addAll(listToAdd);
                    }
                }
            }
        } catch (Throwable ignored) {
        }
    }

    private static Object createLegacyGeocacheFromScraped(ScrapedData scraped) throws Exception {
        Class<?> builderClass = Class.forName("com.groundspeak.geocaching.intro.types.LegacyGeocache$Builder");
        Object builder = builderClass.getDeclaredConstructor().newInstance();
        Class<?> cacheClass = Class.forName("com.groundspeak.geocaching.intro.types.LegacyGeocache");

        builderClass.getField("c").set(builder, scraped.code);
        builderClass.getField("x").set(builder, scraped.name != null ? scraped.name : scraped.code);
        builderClass.getField("y").set(builder, scraped.placedBy != null ? scraped.placedBy : "");
        builderClass.getField("A").set(builder, scraped.shortDescription != null ? scraped.shortDescription : "");
        builderClass.getField("u").set(builder, scraped.longDescription != null ? scraped.longDescription : "");
        builderClass.getField("i").set(builder, scraped.encodedHints != null ? scraped.encodedHints : "");
        builderClass.getField("h").set(builder, scraped.difficulty > 0 ? (double) scraped.difficulty : 1.0);
        builderClass.getField("C").set(builder, scraped.terrain > 0 ? (double) scraped.terrain : 1.0);
        builderClass.getField("K").set(builder, false); // isLite = false
        builderClass.getField("r").set(builder, true);  // available = true
        builderClass.getField("q").set(builder, true);  // isPublished = true
        builderClass.getField("v").set(builder, Boolean.TRUE); // longDescriptionIsHtml
        builderClass.getField("B").set(builder, Boolean.TRUE); // shortDescriptionIsHtml

        return cacheClass.getDeclaredConstructor(builderClass).newInstance(builder);
    }

    private static Object queryLiteGeocacheFromDb(Object repo, String code) {
        try {
            Field fField = repo.getClass().getField("f"); // com.groundspeak.geocaching.intro.geocache.a
            Object geocacheA = fField.get(repo);
            Field dbField = geocacheA.getClass().getField("a"); // GeoDatabase
            Object geoDb = dbField.get(geocacheA);

            Method iMethod = geoDb.getClass().getMethod("i");
            Object helper = iMethod.invoke(geoDb);
            Method k0Method = helper.getClass().getMethod("k0");
            Object sdb = k0Method.invoke(helper);
            Field rawDbField = sdb.getClass().getDeclaredField("A");
            rawDbField.setAccessible(true);
            android.database.sqlite.SQLiteDatabase db = (android.database.sqlite.SQLiteDatabase) rawDbField.get(sdb);

            Cursor cursor = db.rawQuery("SELECT * FROM lite_geocaches WHERE refCode = ?", new String[]{code});
            if (cursor != null) {
                try {
                    if (cursor.moveToFirst()) {
                        return buildLegacyCacheFromLiteCursor(cursor);
                    }
                } finally {
                    cursor.close();
                }
            }
        } catch (Throwable t) {
            Log.d(TAG, "queryLiteGeocacheFromDb error: " + t);
        }
        return null;
    }

    private static Object buildLegacyCacheFromLiteCursor(Cursor cursor) throws Exception {
        Class<?> builderClass = Class.forName("com.groundspeak.geocaching.intro.types.LegacyGeocache$Builder");
        Object builder = builderClass.getDeclaredConstructor().newInstance();
        Class<?> cacheClass = Class.forName("com.groundspeak.geocaching.intro.types.LegacyGeocache");

        String refCode = getCursorString(cursor, "refCode");
        String name = getCursorString(cursor, "name");
        double diff = getCursorDouble(cursor, "difficulty");
        double terr = getCursorDouble(cursor, "terrain");
        int fav = getCursorInt(cursor, "favoritePoints");
        int track = getCursorInt(cursor, "trackableCount");
        long placedDate = getCursorLong(cursor, "placedDate");
        int cacheType = getCursorInt(cursor, "cacheType");
        int containerType = getCursorInt(cursor, "containerType");
        double lat = getCursorDouble(cursor, "latitude");
        double lng = getCursorDouble(cursor, "longitude");
        String placedBy = getCursorString(cursor, "placedBy");
        String ownerUsername = getCursorString(cursor, "owner_username");
        String ownerRefCode = getCursorString(cursor, "owner_referenceCode");

        builderClass.getField("c").set(builder, refCode);
        builderClass.getField("x").set(builder, name);
        builderClass.getField("h").set(builder, diff);
        builderClass.getField("C").set(builder, terr);
        builderClass.getField("j").set(builder, fav);
        builderClass.getField("D").set(builder, track);
        builderClass.getField("t").set(builder, lat);
        builderClass.getField("w").set(builder, lng);
        builderClass.getField("y").set(builder, placedBy != null ? placedBy : ownerUsername);
        if (placedDate > 0) {
            Date d = new Date(placedDate);
            builderClass.getField("e").set(builder, d);
            builderClass.getField("H").set(builder, d);
        }
        builderClass.getField("r").set(builder, true);
        builderClass.getField("q").set(builder, true);

        // Build LatLng
        try {
            Class<?> latLngClass = Class.forName("com.google.android.gms.maps.model.LatLng");
            Object latLng = latLngClass.getDeclaredConstructor(double.class, double.class).newInstance(lat, lng);
            builderClass.getField("V").set(builder, latLng);
        } catch (Throwable ignored) {
        }

        // Build Owner
        try {
            Class<?> ownerClass = Class.forName("com.groundspeak.geocaching.intro.types.LegacyGeocache$Owner");
            Object owner = ownerClass.getDeclaredConstructor(String.class, String.class, String.class)
                    .newInstance(ownerRefCode, ownerUsername, "");
            builderClass.getField("I").set(builder, owner);
        } catch (Throwable ignored) {
        }

        return cacheClass.getDeclaredConstructor(builderClass).newInstance(builder);
    }

    private static String getCursorString(Cursor c, String col) {
        int idx = c.getColumnIndex(col);
        return idx >= 0 ? c.getString(idx) : null;
    }

    private static double getCursorDouble(Cursor c, String col) {
        int idx = c.getColumnIndex(col);
        return idx >= 0 ? c.getDouble(idx) : 0.0;
    }

    private static int getCursorInt(Cursor c, String col) {
        int idx = c.getColumnIndex(col);
        return idx >= 0 ? c.getInt(idx) : 0;
    }

    private static long getCursorLong(Cursor c, String col) {
        int idx = c.getColumnIndex(col);
        return idx >= 0 ? c.getLong(idx) : 0L;
    }

    private static ScrapedData scrapePublicCachePage(String code) {
        HttpURLConnection conn = null;
        try {
            URL url = new URL("https://www.geocaching.com/geocache/" + code);
            conn = (HttpURLConnection) url.openConnection();
            conn.setRequestMethod("GET");
            conn.setConnectTimeout(8000);
            conn.setReadTimeout(8000);
            conn.setRequestProperty("User-Agent", "Mozilla/5.0 (Linux; Android 14; Mobile; rv:128.0) Gecko/128.0 Firefox/128.0");
            conn.setInstanceFollowRedirects(true);
            int respCode = conn.getResponseCode();
            if (respCode != 200) {
                Log.w(TAG, "Public cache page returned HTTP " + respCode + " for " + code);
                return null;
            }
            InputStream in = conn.getInputStream();
            BufferedReader reader = new BufferedReader(new InputStreamReader(in, StandardCharsets.UTF_8));
            StringBuilder sb = new StringBuilder();
            String line;
            while ((line = reader.readLine()) != null) {
                sb.append(line).append('\n');
            }
            reader.close();
            String html = sb.toString();

            if (html.contains("window.isCachePMO = 'Yes'") || html.contains("Premium Member Only")) {
                Log.i(TAG, code + " is a PMO cache; skipping public enrichment");
                return null;
            }

            ScrapedData data = new ScrapedData();
            data.code = code;

            Matcher mOwner = PATTERN_OWNER.matcher(html);
            if (mOwner.find()) {
                data.placedBy = unescapeHtml(mOwner.group(1).trim());
            }

            Matcher mName = PATTERN_NAME.matcher(html);
            if (mName.find()) {
                data.name = unescapeHtml(mName.group(1).trim());
            }

            Matcher mShort = PATTERN_SHORT_DESC.matcher(html);
            if (mShort.find()) {
                data.shortDescription = mShort.group(1).trim();
            } else {
                data.shortDescription = "";
            }

            Matcher mLong = PATTERN_LONG_DESC.matcher(html);
            if (mLong.find()) {
                data.longDescription = mLong.group(1).trim();
            } else {
                data.longDescription = "";
            }

            Matcher mHint = PATTERN_HINT.matcher(html);
            if (mHint.find()) {
                data.encodedHints = unescapeHtml(mHint.group(1).trim());
            } else {
                data.encodedHints = "";
            }

            return data;
        } catch (Throwable t) {
            Log.e(TAG, "Error scraping geocache " + code, t);
            return null;
        } finally {
            if (conn != null) {
                conn.disconnect();
            }
        }
    }

    private static String unescapeHtml(String input) {
        if (input == null || input.isEmpty()) {
            return "";
        }
        String s = input.replace("&amp;", "&")
                .replace("&quot;", "\"")
                .replace("&apos;", "'")
                .replace("&#39;", "'")
                .replace("&lt;", "<")
                .replace("&gt;", ">")
                .replace("&nbsp;", " ");
        if (s.contains("&#")) {
            Matcher m = Pattern.compile("&#(\\d+);").matcher(s);
            StringBuffer sb = new StringBuffer();
            while (m.find()) {
                try {
                    int codePoint = Integer.parseInt(m.group(1));
                    m.appendReplacement(sb, Matcher.quoteReplacement(new String(Character.toChars(codePoint))));
                } catch (Exception e) {
                    m.appendReplacement(sb, m.group(0));
                }
            }
            m.appendTail(sb);
            s = sb.toString();
        }
        return s;
    }

    private static final class ScrapedData {
        String code;
        String name;
        String placedBy;
        String shortDescription;
        String longDescription;
        String encodedHints;
        float difficulty;
        float terrain;
    }
}
