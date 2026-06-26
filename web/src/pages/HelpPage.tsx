// Page d'aide avec navigation entre plusieurs pages de documentation

import { createContext, useContext, type ReactNode } from 'react';
import { useState } from 'react';

interface HelpPage {
  id: string;
  title: string;
  content: ReactNode;
}

// Contexte de navigation entre pages d'aide
const HelpNavContext = createContext<(page: string) => void>(() => {});

// Composant de lien vers une autre page d'aide
interface NavLinkProps {
  to: string;
  children: ReactNode;
}

function NavLink({ to, children }: NavLinkProps) {
  const onNavigate = useContext(HelpNavContext);

  const onClick = () => {
    onNavigate(to);
    window.scrollTo({ top: 0, behavior: 'smooth' });
  };

  return (
    <button type="button" className="help-nav-link-inline" onClick={onClick}>
      {children}
    </button>
  );
}

// Contenu de la première page : Postes et Indicateurs
const postesIndicateursContent = (
  <div className="help-content">
    <h2>Vue d'ensemble</h2>
    <p>
      La vue Calcul regroupe deux objets métier fondamentaux : <strong>Postes</strong> et <strong>Indicateurs</strong>.
      Ce sont les briques du moteur de formules qui permettent de créer des mesures dérivées à partir des données consolidées.
    </p>

    <h2>1. Postes (dim_aggregate)</h2>
    <h3>Définition</h3>
    <p>
      Un <strong>Poste</strong> est une sélection nommée sur la table <code>fact_entry</code>, agrégée en un montant signé.
      C'est la <strong>brique de base</strong> du moteur de calcul.
    </p>

    <h3>Structure</h3>
    <ul>
      <li><code>code</code> : identifiant unique du poste (ex. <code>ca</code>, <code>stocks</code>)</li>
      <li><code>libellé</code> : description lisible par l'utilisateur</li>
      <li><code>level</code> : niveau de consolidation (<code>corporate</code> | <code>converted</code> | <code>consolidated</code>)</li>
      <li><code>definition</code> : JSON contenant les conditions de sélection</li>
    </ul>

    <h3>Définition d'un poste</h3>
    <p>Un poste se définit par :</p>
    <ul>
      <li>Un <strong>niveau</strong> de consolidation (obligatoire)</li>
      <li>Une <strong>sélection</strong> de conditions dimensionnelles (optionnelle)</li>
    </ul>

    <p>Chaque condition peut spécifier :</p>
    <ul>
      <li><code>dim</code> : dimension filtrée (ex. <code>account</code>, <code>entity</code>, <code>flow</code>)</li>
      <li><code>op</code> : opérateur (<code>=</code>, <code>!=</code>, <code>&gt;</code>, <code>&lt;</code>, <code>IN</code>, <code>IS NULL</code>, etc.)</li>
      <li><code>val</code> : valeur de filtrage</li>
      <li><code>via</code> : caractéristique de niveau N1 (ex. <code>comportement</code> sur <code>account</code>)</li>
      <li><code>ref</code> : référence directe (patron B)</li>
      <li><code>attr</code> : attribut natif (ex. <code>classe</code>, <code>sous_classe</code>)</li>
    </ul>

    <h3>Exemple de définition</h3>
    <pre>
{`{
  "level": "consolidated",
  "selection": [
    { "dim": "account", "op": "=", "val": "700", "attr": "classe" },
    { "dim": "flow", "op": "=", "val": "F99" }
  ]
}`}
    </pre>
    <p>
      Sélectionne toutes les écritures du niveau <code>consolidated</code> où le compte appartient à la classe <code>700</code> (ventes)
      et le flux est <code>F99</code> (clôture).
    </p>

    <h3>Compilation SQL</h3>
    <p>Un poste se compile en un agrégat conditionnel :</p>
    <pre>
{`SUM(e.amount) FILTER (WHERE e.level = 'consolidated'
                       AND imda_account_classe.classe = ?
                       AND e.flow = ?)`}
    </pre>
    <p>
      Les traversées (<code>via</code>/<code>ref</code>/<code>attr</code>) ajoutent des <strong>LEFT JOINs partagés</strong>
      (un poste ne filtre pas les lignes des autres postes).
    </p>

    <h3>Utilisation</h3>
    <p>Les postes sont des <strong>opérandes</strong> insérables dans les formules d'indicateurs via la syntaxe <code>[code_poste]</code>.</p>

    <h2>2. Indicateurs (dim_indicator)</h2>
    <h3>Définition</h3>
    <p>
      Un <strong>Indicateur</strong> est une <strong>formule</strong> combinant des postes et d'autres indicateurs,
      calculée à un <strong>grain de restitution</strong>. C'est une mesure dérivée (marge, ROE, ratio d'endettement…).
    </p>

    <h3>Structure</h3>
    <ul>
      <li><code>code</code> : identifiant unique de l'indicateur (ex. <code>marge_op</code>, <code>roe</code>)</li>
      <li><code>libellé</code> : description lisible</li>
      <li><code>expression</code> : formule (langage §2)</li>
      <li><code>grain</code> : tableau de dimensions de restitution (ex. <code>["entity"]</code>, <code>["entity", "period"]</code>)</li>
      <li><code>format</code> : format d'affichage (<code>nombre</code> | <code>pourcentage</code> | <code>ratio</code>)</li>
    </ul>

    <h3>Langage de formules</h3>
    <p>Syntaxe proche d'Excel :</p>
    <ul>
      <li><strong>Opérateurs</strong> : <code>+</code>, <code>-</code>, <code>×</code>, <code>÷</code> (ou <code>*</code>, <code>/</code>)</li>
      <li><strong>Références</strong> : <code>[code_poste]</code> ou <code>[code_indicateur]</code></li>
      <li><strong>Fonctions</strong> : <code>MIN</code>, <code>MAX</code>, <code>ABS</code>, <code>ROUND</code>, <code>IF</code>, <code>SAFE_DIV</code></li>
      <li><strong>Séparateur d'arguments</strong> : <code>;</code></li>
    </ul>

    <h3>Exemples</h3>
    <pre>
{`Marge opérationnelle = SAFE_DIV([resultat] ; [ca])
Ratio d'endettement  = SAFE_DIV([dettes_fin] ; [capitaux_propres])
Croissance CA        = SAFE_DIV([ca] - [ca_n1] ; [ca_n1])`}
    </pre>

    <h3>Compilation SQL</h3>
    <p>Pour un grain donné (ex. <code>entity</code>), l'indicateur se compile en <strong>une seule requête ensembliste</strong> :</p>
    <pre>
{`SELECT entity,
       SAFE_DIVIDE( SUM(e.amount) FILTER (WHERE <sél. résultat>),
                    SUM(e.amount) FILTER (WHERE <sél. ca>) ) AS marge_op
FROM fact_entry e
WHERE e.consolidation_id = ?
GROUP BY entity`}
    </pre>
    <p>Chaque poste devient un <code>SUM(amount) FILTER (WHERE …)</code>, la formule devient de l'arithmétique dans le <code>SELECT</code>.</p>

    <h3>Non-additivité</h3>
    <p>Les indicateurs sont <strong>non additifs</strong> par nature :</p>
    <ul>
      <li>Un ratio ne se somme pas (on ne fait pas la somme des pourcentages)</li>
      <li>L'indicateur est <strong>recalculé</strong> pour chaque niveau d'agrégation</li>
      <li><strong>Jamais réinjecté</strong> dans <code>fact_entry</code> — couche de présentation uniquement</li>
    </ul>

    <h2>3. Relation entre Postes et Indicateurs</h2>
    <pre>
{`Postes (dim_aggregate)
   ↓
Sélections nommées sur fact_entry
   ↓
Opérandes des indicateurs [code_poste]
   ↓
Indicateurs (dim_indicator)
   ↓
Formules combinant des postes
   ↓
Résultats affichés (rapports, dashboard)`}
    </pre>
    <ul>
      <li>Les <strong>postes</strong> sont des <strong>briques de base</strong> (agrégats nommés)</li>
      <li>Les <strong>indicateurs</strong> sont des <strong>formules</strong> qui combinent ces briques</li>
      <li>Les indicateurs peuvent <strong>référencer d'autres indicateurs</strong> (avec détection de cycle)</li>
    </ul>

    <h2>4. Persistance et API</h2>
    <h3>Stockage</h3>
    <p>Les deux objets survivent au reset de la base (registre hors <code>ALL_DROP</code>) :</p>
    <ul>
      <li><code>dim_aggregate</code> : <code>code</code>, <code>libellé</code>, <code>level</code>, <code>definition</code> (JSON)</li>
      <li><code>dim_indicator</code> : <code>code</code>, <code>libellé</code>, <code>expression</code>, <code>grain</code> (JSON), <code>format</code></li>
    </ul>

    <h3>Routes API</h3>
    <ul>
      <li><code>GET/POST /api/aggregates</code> — CRUD des postes</li>
      <li><code>PUT/DELETE /api/aggregates/{`{code}`}</code></li>
      <li><code>GET/POST /api/indicators</code> — CRUD des indicateurs</li>
      <li><code>PUT/DELETE /api/indicators/{`{code}`}</code></li>
      <li><code>GET /api/indicators/operands</code> — catalogue des postes + indicateurs insérables</li>
      <li><code>POST /api/indicators/preview</code> — évaluation sans sauvegarde</li>
    </ul>

    <h2>5. UI et ergonomie</h2>
    <h3>Page Postes</h3>
    <ul>
      <li>Liste des postes existants (code, niveau)</li>
      <li>Création/édition via un éditeur de conditions</li>
      <li>Résumé textuel de la sélection (ex. « Sur consolidated, somme des montants des écritures où account.comportement = 'VENTES'… »)</li>
    </ul>

    <h3>Page Indicateurs</h3>
    <ul>
      <li>Liste des indicateurs existants</li>
      <li>Éditeur de formules avec autocomplétion</li>
      <li>Palette d'opérandes (postes + indicateurs)</li>
      <li>Sélection du grain de restitution (chips par dimension)</li>
      <li>Preview live sur une consolidation</li>
      <li>Format d'affichage (nombre, %, ratio)</li>
    </ul>

    <h2>6. Sécurité SQL</h2>
    <p>
      Identifiants (dimensions, <code>via</code>/<code>ref</code>/<code>attr</code>, <code>level</code>, grain) <strong>validés contre des whitelists</strong> dérivées du registre.
      Seules les valeurs utilisateur passent par des paramètres <code>?</code> — aucune interpolation brute.
    </p>
  </div>
);

const coefficientsContent = (
  <div className="help-content">
    <h2>Vue d'ensemble</h2>
    <p>
      Les <strong>coefficients</strong> sont des formules nommées qui calculent des valeurs scalaires à partir des taux de périmètre. Ils sont utilisés dans les opérations de règles de consolidation pour appliquer des facteurs dynamiques (ex. pourcentage d'intégration, pourcentage d'intérêt, élimination interco).
    </p>

    <h2>1. Définition</h2>
    <p>
      Un coefficient est une <strong>formule</strong> qui renvoie un scalaire (nombre) calculé à partir des données de périmètre (`sat_perimeter`). Il est évalué au grain d'une écriture source d'une opération de règle.
    </p>

    <h3>Structure</h3>
    <ul>
      <li><code>code</code> : identifiant unique du coefficient (ex. <code>pct_integration</code>, <code>elim_ic_corp_n</code>)</li>
      <li><code>libellé</code> : description lisible (optionnel)</li>
      <li><code>expression</code> : formule (langage §2)</li>
      <li><code>kind</code> : type (<code>builtin</code> | <code>user</code>)</li>
    </ul>

    <h2>2. Langage de formules</h2>
    <p>Syntaxe proche d'Excel :</p>
    <ul>
      <li><strong>Opérateurs</strong> : <code>+</code>, <code>-</code>, <code>×</code>, <code>÷</code> (ou <code>*</code>, <code>/</code>)</li>
      <li><strong>Références</strong> : <code>[pct_integration.entity]</code>, <code>[pct_interet.partner]</code>, etc.</li>
      <li><strong>Fonctions</strong> : <code>MIN</code>, <code>MAX</code>, <code>ABS</code>, <code>ROUND</code>, <code>IF</code>, <code>SAFE_DIV</code></li>
      <li><strong>Séparateur d'arguments</strong> : <code>;</code></li>
    </ul>

    <h2>3. Catalogue d'opérandes (périmètre)</h2>
    <p>Les références <code>[ … ]</code> pointent vers des valeurs de périmètre lues sur <code>sat_perimeter</code>, à l'une des <strong>quatre perspectives</strong> :</p>

    <table className="help-table">
      <thead>
        <tr>
          <th>Perspective</th>
          <th>Entité lue</th>
          <th>Période</th>
        </tr>
      </thead>
      <tbody>
        <tr><td><code>entity</code></td><td>l'entité de l'écriture</td><td>courante</td></tr>
        <tr><td><code>partner</code></td><td>le partenaire de l'écriture</td><td>courante</td></tr>
        <tr><td><code>entity_n1</code></td><td>l'entité</td><td>N-1 (via à-nouveau)</td></tr>
        <tr><td><code>partner_n1</code></td><td>le partenaire</td><td>N-1 (via à-nouveau)</td></tr>
      </tbody>
    </table>

    <p>Champs disponibles par perspective : <code>pct_integration</code>, <code>pct_interet</code> (et tout champ numérique de `sat_perimeter` whitelisté).</p>

    <h3>Références disponibles</h3>
    <pre>
{`[pct_integration.entity]   → COALESCE(p_ent.pct_integration, 0)
[pct_integration.partner]   → COALESCE(p_part.pct_integration, 0)
[pct_integration.entity_n1] → COALESCE(p_ent_n1.pct_integration, 0)
[pct_interet.entity]       → COALESCE(p_ent.pct_interet, 0)
[pct_interet.partner]       → COALESCE(p_part.pct_interet, 0)`}
    </pre>

    <h2>4. Défaut uniforme = 0</h2>
    <p>Tout taux de périmètre absent (entité ou partenaire hors périmètre, perspective N-1 d'une entité entrante) vaut <strong>0</strong>. Conséquences :</p>
    <ul>
      <li>Un coefficient <code>pct_integration</code> posé seul sur une écriture dont l'entité est hors périmètre annule l'écriture (× 0).</li>
      <li>La vigilance « n'utiliser un coefficient à partenaire que là où il y a un partenaire » relève de <strong>l'utilisateur</strong>.</li>
    </ul>

    <h2>5. Coefficients natifs</h2>
    <p>Les coefficients natifs sont seedés comme formules prédéfinies dans la bibliothèque :</p>

    <table className="help-table">
      <thead>
        <tr>
          <th>Code</th>
          <th>Expression</th>
          <th>Description</th>
        </tr>
      </thead>
      <tbody>
        <tr><td><code>pct_integration</code></td><td><code>[pct_integration.entity]</code></td><td>Pourcentage d'intégration de l'entité</td></tr>
        <tr><td><code>pct_interet</code></td><td><code>[pct_interet.entity]</code></td><td>Pourcentage d'intérêt de l'entité</td></tr>
        <tr><td><code>elim_ic_corp_n</code></td><td><code>MIN(1; SAFE_DIV([pct_integration.partner]; [pct_integration.entity]))</code></td><td>Élimination interco au prorata du plus faible taux d'intégration N</td></tr>
        <tr><td><code>elim_ic_corp_n1</code></td><td><code>MIN(1; SAFE_DIV([pct_integration.partner_n1]; [pct_integration.entity_n1]))</code></td><td>Élimination interco N-1</td></tr>
        <tr><td><code>elim_ic_corp_var</code></td><td><code>MIN(1; SAFE_DIV([pct_integration.partner]; [pct_integration.entity])) - MIN(1; SAFE_DIV([pct_integration.partner_n1]; [pct_integration.entity_n1]))</code></td><td>Variation de l'élimination interco</td></tr>
      </tbody>
    </table>

    <h2>6. Coefficients utilisateur</h2>
    <p>L'utilisateur peut créer ses propres coefficients via l'éditeur de formules. Exemples :</p>
    <pre>
{`minoritaire = 1 - [pct_interet.entity]
ecart_integ_int = [pct_interet.entity] - [pct_integration.entity]`}
    </pre>

    <h2>7. Utilisation dans les règles</h2>
    <p>Dans une opération de règle, le coefficient est multiplié au montant :</p>
    <pre>
{`facteur = coefficient × multiplicateur`}
    </pre>
    <ul>
      <li><strong>coefficient</strong> : valeur dynamique issue du périmètre (référence à la bibliothèque ou <code>constant</code> inline)</li>
      <li><strong>multiplicateur</strong> : constante (typiquement 1 ou −1)</li>
    </ul>

    <h2>8. Persistance</h2>
    <ul>
      <li>Table <code>dim_coefficient</code> : <code>code</code>, <code>libellé</code>, <code>expression</code>, <code>kind</code></li>
      <li>Les coefficients natifs sont seedés comme <code>builtin</code> (non modifiables)</li>
      <li>Les coefficients utilisateur sont <code>user</code> (modifiables en place)</li>
      <li>Modifiable en place : l'édition met à jour toutes les règles qui le référencent</li>
    </ul>

    <h2>9. API</h2>
    <ul>
      <li><code>GET /api/coefficients</code> — liste la bibliothèque (natifs + utilisateur)</li>
      <li><code>POST /api/coefficients</code> — crée un coefficient utilisateur</li>
      <li><code>PUT /api/coefficients/{`{code}`}</code> — modifie un coefficient utilisateur</li>
      <li><code>DELETE /api/coefficients/{`{code}`}</code> — supprime un coefficient utilisateur</li>
      <li><code>GET /api/coefficients/operands</code> — catalogue des opérandes (token + label)</li>
      <li><code>POST /api/coefficients/preview</code> — évaluation sans sauvegarde</li>
    </ul>

    <h2>10. Compilation SQL</h2>
    <p>Un coefficient se compile en une expression SQL scalaire avec jointures de périmètre :</p>
    <pre>
{`COALESCE(p_ent.pct_integration, 0)
CASE WHEN (p_ent.pct_integration) = 0 THEN 0 ELSE (p_part.pct_integration) / (p_ent.pct_integration) END`}
    </pre>
    <p>Les jointures (<code>p_ent</code>, <code>p_part</code>, <code>p_ent_n1</code>, <code>p_part_n1</code>) sont ajoutées dynamiquement selon les perspectives utilisées.</p>

    <h2>11. UI et ergonomie</h2>
    <h3>Page Coefficients</h3>
    <ul>
      <li>Liste des coefficients existants (code, type natif/utilisateur)</li>
      <li>Création/édition via un éditeur de formules</li>
      <li>Palette d'opérandes insérables (champs de périmètre × 4 perspectives)</li>
      <li>Fonctions insérables : <code>MIN</code>, <code>MAX</code>, <code>ABS</code>, <code>ROUND</code>, <code>IF</code>, <code>SAFE_DIV</code></li>
      <li>Preview live avec valeurs d'exemple</li>
      <li>Duplication possible des natifs pour créer des variantes utilisateur</li>
    </ul>

    <h2>12. Sécurité SQL</h2>
    <p>
      Noms de champs et perspectives validés par whitelists dérivées du registre. Seules les constantes numériques sont émises comme littéraux SQL. Aucun identifiant utilisateur n'est interpolé brut.
    </p>

    <h3>Voir aussi</h3>
    <ul>
      <li><NavLink to='regles'>Utilisation des coefficients dans les règles</NavLink></li>
    </ul>
  </div>
);

const reglesContent = (
  <div className="help-content">
    <h2>Vue d'ensemble</h2>
    <p>
      L'<strong>éditeur de règles</strong> permet de composer des écritures automatiques pour les traitements de consolidation non couverts par les natifs du moteur : éliminations interco, éliminations de participations, intérêts minoritaires, retraitements, variations de capital, répartition des résultats.
    </p>

    <h2>1. Modèle d'une règle</h2>
    <pre>
{`RÈGLE
├── Identité : code, libellé
├── Scope périmètre : conditions sur sat_perimeter
└── Opérations : 1 à N, exécutées dans l'ordre`}
    </pre>

    <p>Une règle = <strong>un scope périmètre</strong> partagé + <strong>N opérations</strong>. Chaque opération a sa propre sélection, son propre facteur, et sa propre destination. Les opérations d'une même règle partagent le même scope.</p>

    <h2>2. Scope périmètre</h2>
    <p>Définit <strong>à quelles entités</strong> la règle s'applique, par filtrage sur les attributs de <code>sat_perimeter</code> :</p>

    <table className="help-table">
      <thead>
        <tr>
          <th>Target</th>
          <th>Dim</th>
          <th>Exemples de conditions</th>
        </tr>
      </thead>
      <tbody>
        <tr><td><code>entity</code></td><td><code>methode</code></td><td>= globale, = proportionnelle, IN (globale, proportionnelle)</td></tr>
        <tr><td><code>entity</code></td><td><code>pct_interet</code></td><td>{`>`} 0, = 0.5</td></tr>
        <tr><td><code>entity</code></td><td><code>pct_integration</code></td><td>{`>`} 0, = 1.0</td></tr>
        <tr><td><code>entity</code></td><td><code>entree</code></td><td>= true (entités entrantes)</td></tr>
        <tr><td><code>entity</code></td><td><code>sortie</code></td><td>= true (entités sortantes)</td></tr>
        <tr><td><code>partner</code></td><td><code>methode</code></td><td>= globale (scope croisé pour éliminations interco)</td></tr>
        <tr><td><code>share</code></td><td><code>methode</code></td><td>= globale (scope sur participations)</td></tr>
      </tbody>
    </table>

    <h3>Scope croisé</h3>
    <p>Pour les éliminations interco, le scope peut porter sur <strong>deux entités simultanément</strong> — l'entité source et le partenaire. Ex : « l'entité ET son partenaire sont tous deux en méthode globale ».</p>

    <h3>Articulation des conditions</h3>
    <p>Les conditions du scope sont combinées exclusivement par <strong>ET</strong>. Pour exprimer un <strong>OU</strong> sur une même dimension, utiliser l'opérateur <code>IN</code>.</p>

    <h2>3. Modèle d'une opération</h2>
    <p>Chaque opération a trois composantes : <strong>Sélection → Facteur → Destination</strong>.</p>

    <h3>3.1 Sélection</h3>
    <p>Cible un sous-ensemble de grains dans <code>fact_entry</code> en filtrant sur <strong>toutes les dimensions</strong> disponibles.</p>

    <h4>Niveau de sélection</h4>
    <p>La sélection se fait à un niveau de stockage donné. Ce niveau détermine le niveau d'écriture des entrées générées :</p>
    <ul>
      <li><code>corporate</code> : données saisies agrégées par entité</li>
      <li><code>converted</code> : données après conversion multi-devises</li>
      <li><code>consolidated</code> : données après application des méthodes</li>
    </ul>

    <h4>Modes de sélection</h4>
    <ul>
      <li><strong>Direct</strong> : filtre sur la valeur directe de la dimension (ex : <code>flow = 'F99'</code>, <code>partner IS NOT NULL</code>)</li>
      <li><strong>Par caractéristique N1</strong> (via) : filtre sur la valeur N1 du membre (ex : <code>account.comportement = 'VENTES_IC'</code>)</li>
      <li><strong>Par référence directe</strong> (ref) : filtre sur une colonne de référence (ex : <code>account.compte_parent = '60'</code>)</li>
      <li><strong>Par enum natif</strong> (attr) : filtre sur un attribut natif (CHECK du DDL, ex : <code>account.classe = 'bilan'</code>)</li>
    </ul>

    <h4>Traversées mutuellement exclusives</h4>
    <p>Une condition peut utiliser <strong>un seul</strong> mode de traversée parmi <code>via</code>, <code>ref</code> ou <code>attr</code>. Ces trois champs sont mutuellement exclusifs.</p>

    <h4>Opérateurs autorisés</h4>
    <p><code>=</code>, <code>!=</code>, <code>{`>`}</code>, <code>{`<`}</code>, <code>{`>=`}</code>, <code>{`<=`}</code>, <code>IN</code>, <code>IS NULL</code>, <code>IS NOT NULL</code></p>

    <h3>3.2 Facteur</h3>
    <p>Le facteur appliqué au montant de chaque grain sélectionné :</p>
    <pre>
{`facteur = coefficient × multiplicateur`}
    </pre>

    <h4>Coefficient</h4>
    <p>Valeur dynamique issue du périmètre ou d'un taux :</p>
    <ul>
      <li><strong>Référence à la bibliothèque</strong> : code d'un coefficient de <code>dim_coefficient</code> (natif ou utilisateur)</li>
      <li><strong>Constant</strong> : littéral inline (<code>{`{"type":"constant","value":…}`}</code>)</li>
    </ul>

    <h4>Coefficients natifs (seedés)</h4>
    <table className="help-table">
      <thead>
        <tr>
          <th>Code</th>
          <th>Type</th>
          <th>Description</th>
        </tr>
      </thead>
      <tbody>
        <tr><td><code>pct_integration</code></td><td>Named</td><td>Pourcentage d'intégration de l'entité</td></tr>
        <tr><td><code>pct_interet</code></td><td>Named</td><td>Pourcentage d'intérêt de l'entité</td></tr>
        <tr><td><code>elim_ic_corp_n</code></td><td>Named</td><td>Élimination interco au prorata du plus faible taux d'intégration N</td></tr>
        <tr><td><code>elim_ic_corp_n1</code></td><td>Named</td><td>Élimination interco N-1</td></tr>
        <tr><td><code>elim_ic_corp_var</code></td><td>Named</td><td>Variation de l'élimination interco</td></tr>
        <tr><td><code>constant</code></td><td>Constant</td><td>Littéral inline (valeur saisie)</td></tr>
      </tbody>
    </table>

    <h4>Multiplicateur</h4>
    <p>Constante, typiquement 1 ou −1. Défaut implicite = 1 si non spécifié.</p>

    <h3>3.3 Destination</h3>
    <p>Définit où et comment écrire l'écriture générée. Pour <strong>chaque dimension pilotable</strong>, cinq modes possibles :</p>

    <table className="help-table">
      <thead>
        <tr>
          <th>Mode</th>
          <th>Sémantique</th>
          <th>Champs requis</th>
        </tr>
      </thead>
      <tbody>
        <tr><td><code>inherit</code></td><td>La valeur du grain source est conservée.</td><td>Aucun</td></tr>
        <tr><td><code>override</code></td><td>La valeur est forcée à une constante saisie.</td><td><code>value</code></td></tr>
        <tr><td><code>null</code></td><td>La valeur est vidée (<code>NULL</code>).</td><td>Aucun</td></tr>
        <tr><td><code>map</code></td><td>La valeur est résolue en traversant une caractéristique N1 (<code>via</code>) pour lire un attribut N2 (<code>attr</code>).</td><td><code>via</code>, <code>attr</code></td></tr>
        <tr><td><code>map_ref</code></td><td>La valeur est résolue en traversant une référence directe (<code>ref</code>) portée par la dimension écrite.</td><td><code>ref</code></td></tr>
      </tbody>
    </table>

    <h4>Dimensions toujours héritées (non pilotables)</h4>
    <p><code>phase</code>, <code>scenario</code>, <code>entry_period</code>, <code>period</code>, <code>currency</code></p>

    <h4>Dimensions pilotables</h4>
    <p><strong>Built-in :</strong> <code>entity</code>, <code>account</code>, <code>flow</code>, <code>nature</code>, <code>partner</code>, <code>share</code>, <code>analysis</code>, <code>analysis2</code></p>
    <p><strong>Custom :</strong> toutes les dimensions custom créées par l'utilisateur</p>

    <h2>4. Sécurité SQL</h2>
    <p>Les identifiants (dimensions, opérateurs, modes de destination, level) sont validés contre des whitelists dérivées du registre central des dimensions. Seules les valeurs utilisateur passent par des paramètres <code>?</code> — aucune interpolation brute.</p>

    <h2>5. Exemple : élimination interco</h2>
    <h3>Contexte</h3>
    <p>Une écriture interco est identifiée par la présence d'une valeur dans la dimension <code>partner</code>. Le solde interco doit être extourné au niveau consolidé.</p>

    <h3>Règle « Élimination interco »</h3>
    <p><strong>Scope périmètre</strong> : <code>entity.methode = 'globale' AND partner.methode = 'globale'</code> (scope croisé)</p>
    <p><strong>Niveau de sélection</strong> : consolidated</p>

    <h4>Opérations (4)</h4>
    <table className="help-table">
      <thead>
        <tr>
          <th>Op</th>
          <th>Sélection</th>
          <th>Coefficient</th>
          <th>Multiplicateur</th>
          <th>Destination</th>
        </tr>
      </thead>
      <tbody>
        <tr><td>1</td><td><code>partner IS NOT NULL</code></td><td><code>pct_integration</code></td><td>−1</td><td>nature → <code>2ELI</code>, partner → inherit</td></tr>
        <tr><td>2</td><td><code>partner IS NOT NULL</code></td><td><code>pct_integration</code></td><td>−1</td><td>nature → <code>2ELI</code>, partner → null</td></tr>
        <tr><td>3</td><td><code>partner IS NOT NULL</code></td><td><code>pct_integration</code></td><td>+1</td><td>nature → <code>2ELI</code>, account → compte regroupement, partner → inherit</td></tr>
        <tr><td>4</td><td><code>partner IS NOT NULL</code></td><td><code>pct_integration</code></td><td>+1</td><td>nature → <code>2ELI</code>, account → compte regroupement, partner → null</td></tr>
      </tbody>
    </table>

    <h2>6. Jeux de règles (rulesets)</h2>
    <h3>Modèle</h3>
    <ul>
      <li><strong>Bibliothèque de règles</strong> : ensemble central de règles, chacune avec un code unique. Une règle est immuable dès lors qu'elle est référencée par un jeu.</li>
      <li><strong>Jeu de règles</strong> : collection ordonnée de références vers des règles de la bibliothèque (<code>dim_ruleset</code> + items dans <code>dim_ruleset_item</code>).</li>
      <li><strong>Duplication</strong> : créer un nouveau jeu en copiant les références d'un jeu existant.</li>
      <li><strong>Modification</strong> : pour changer une règle dans un nouveau jeu, l'utilisateur crée une copie de la règle (nouveau code) et la référence dans le nouveau jeu.</li>
    </ul>

    <h3>Exécution</h3>
    <ul>
      <li>La consolidation pointe vers un jeu de règles précis (<code>consolidation.ruleset_code</code>).</li>
      <li>Les règles du jeu sont exécutées séquentiellement dans l'ordre défini (attribut <code>ordre</code> dans <code>dim_ruleset_item</code>).</li>
      <li>Cascade entre règles : la règle N+1 voit les écritures générées par la règle N.</li>
      <li>Les opérations d'une même règle sont indépendantes : toutes sélectionnent sur le même état initial.</li>
    </ul>

    <h2>7. API REST</h2>
    <ul>
      <li><code>GET /api/rules</code> — liste des règles (résumé)</li>
      <li><code>POST /api/rules</code> — créer une règle</li>
      <li><code>PUT /api/rules/{`{code}`}</code> — mettre à jour une règle</li>
      <li><code>DELETE /api/rules/{`{code}`}</code> — supprimer une règle</li>
      <li><code>GET /api/rules/{`{code}`}</code> — détail d'une règle (définition JSON complète)</li>
      <li><code>GET /api/rulesets</code> — liste des jeux de règles (résumé)</li>
      <li><code>POST /api/rulesets</code> — créer un jeu de règles</li>
      <li><code>PUT /api/rulesets/{`{code}`}</code> — mettre à jour un jeu de règles</li>
      <li><code>DELETE /api/rulesets/{`{code}`}</code> — supprimer un jeu de règles</li>
      <li><code>GET /api/rulesets/{`{code}`}</code> — détail d'un jeu de règles (items ordonnés)</li>
      <li><code>POST /api/rulesets/{`{code}`}/execute</code> — exécuter un jeu de règles → rapport</li>
    </ul>

    <h2>8. Interface utilisateur</h2>
    <p>Page <strong>Règles</strong> avec trois sous-onglets :</p>
    <ul>
      <li><strong>Bibliothèque</strong> : CRUD sur les règles (définition JSON éditoriale)</li>
      <li><strong>Jeux de règles</strong> : CRUD sur les rulesets + exécution → rapport</li>
      <li><strong>Dimensions</strong> : gestion des dimensions custom (catégorie Analytical)</li>
    </ul>

    <h3>Éditeur de règle (modale)</h3>
    <p>Chaque règle se définit dans une modale unique. Le niveau d'exécution est commun à toutes les opérations d'une règle (modifiable en haut du formulaire).</p>
    <ul>
      <li><strong>Code</strong> : identifiant unique (éditable à la création uniquement)</li>
      <li><strong>Libellé</strong> : description lisible</li>
      <li><strong>Niveau d'exécution</strong> : corporate / converted / consolidated</li>
      <li><strong>Scope périmètre</strong> : conditions sur sat_perimeter (target, dim, op, valeur)</li>
      <li><strong>Opérations</strong> : sous-formulaires répétables avec sélection, facteur, destination</li>
    </ul>

    <h3>Sous-formulaire opération</h3>
    <p>Pour chaque opération, trois blocs :</p>
    <ul>
      <li><strong>Sélection</strong> : conditions dimensionnelles (dim, traversée, op, valeur)</li>
      <li><strong>Facteur</strong> : coefficient (dropdown + valeur si constant) + multiplicateur</li>
      <li><strong>Destination</strong> : une ligne par dimension pilotable avec mode et champs conditionnels</li>
    </ul>

    <h3>Voir aussi</h3>
    <ul>
      <li><NavLink to='coefficients'>Coefficients (facteurs d'opérations)</NavLink></li>
      <li><NavLink to='taux-integration'>Taux d'intégration natif</NavLink></li>
    </ul>
  </div>
);

const schemasFluxContent = (
  <div className="help-content">
    <h2>Vue d'ensemble</h2>
    <p>
      La consolidation est <strong>par les flux</strong> : chaque traitement de consolidation agit sur des <strong>flux de variation</strong> et génère des écritures taguées avec un code de flux. Le code de flux explicite l'origine de chaque montant → <strong>traçabilité totale</strong>.
    </p>

    <h2>1. Niveaux d'élaboration</h2>
    <p>La consolidation distingue deux concepts :</p>
    <ul>
      <li><strong>Niveaux de stockage</strong> (3) : où les données vivent dans la base</li>
      <li><strong>Étapes de traitement</strong> (3) : l'ordre dans lequel le moteur calcule ces niveaux</li>
    </ul>

    <h3>Niveaux de stockage</h3>
    <table className="help-table">
      <thead>
        <tr>
          <th>Niveau</th>
          <th>Devise</th>
          <th>Contenu</th>
        </tr>
      </thead>
      <tbody>
        <tr><td><strong>Corporate</strong></td><td>Fonctionnelle</td><td>Données saisies agrégées par entité, brutes</td></tr>
        <tr><td><strong>Converti</strong></td><td>Présentation</td><td>Données converties + écarts générés + clôture reconstruite</td></tr>
        <tr><td><strong>Consolidé</strong></td><td>Présentation</td><td>Données après application des méthodes + clôture reconstruite</td></tr>
      </tbody>
    </table>

    <h3>Étapes de traitement</h3>
    <table className="help-table">
      <thead>
        <tr>
          <th>Étape</th>
          <th>Opération</th>
          <th>Entrée</th>
          <th>Sortie</th>
        </tr>
      </thead>
      <tbody>
        <tr><td><strong>A. Agrégation</strong></td><td>Cumul des écritures source par entité</td><td>CSV / saisie</td><td>Corporate</td></tr>
        <tr><td><strong>B. Conversion</strong></td><td>Conversion multi-devises + écarts</td><td>Corporate</td><td>Converti</td></tr>
        <tr><td><strong>C. Consolidation</strong></td><td>Application des méthodes + règles</td><td>Converti</td><td>Consolidé</td></tr>
      </tbody>
    </table>

    <h2>2. Dimension Flow</h2>
    <p><code>Flow</code> (<code>dim_flow</code>) est une dimension nue (<code>code</code>, <code>libellé</code>). Tout le comportement d'un flux est déporté dans le <strong>schéma de flux</strong>.</p>

    <h3>Tables</h3>
    <ul>
      <li><code>dim_flow</code> : catalogue des flux (code, libellé)</li>
      <li><code>dim_flow_scheme</code> : catalogue des schémas de flux (code, libellé)</li>
      <li><code>sat_flow_scheme_item</code> : articulation complète des flux d'un schéma</li>
    </ul>

    <h3>Attributs portés par le schéma</h3>
    <table className="help-table">
      <thead>
        <tr>
          <th>Attribut</th>
          <th>Type</th>
          <th>Rôle</th>
        </tr>
      </thead>
      <tbody>
        <tr><td><code>taux_conversion</code></td><td>TEXT</td><td>Type de taux à appliquer (<code>close_n1</code>, <code>avg</code>, <code>close_n</code>)</td></tr>
        <tr><td><code>flux_ecart</code></td><td>TEXT (nullable)</td><td>Flux d'écart qui recevra la différence de conversion (NULL = aucun écart)</td></tr>
        <tr><td><code>flux_de_report</code></td><td>TEXT (nullable)</td><td>Flux de clôture où ce flux s'agrège (auto-référence = clôture reconstruite)</td></tr>
        <tr><td><code>flux_a_nouveau</code></td><td>TEXT (nullable)</td><td>Flux d'ouverture qui reçoit ce solde à l'exercice suivant</td></tr>
      </tbody>
    </table>

    <h3>Vue de comportement</h3>
    <p>La résolution <code>compte → schéma → comportement</code> passe par la vue <code>v_flow_behavior(account, flow, …)</code>, consommée par <code>pipeline::convert</code>, <code>materialize_closures</code> et <code>a_nouveau</code>.</p>

    <h2>3. Mécanique de conversion</h2>
    <p>Tous les flux sont saisis en <strong>devise fonctionnelle</strong> et convertis via leur <code>taux_conversion</code>.</p>

    <p>Pour un flux X (montant <code>A_X</code> en devise fonctionnelle, taux <code>r_X</code>) :</p>
    <ul>
      <li>Montant converti = <code>A_X × r_X</code></li>
      <li><strong>Écart de conversion</strong> = <code>A_X × (r_report − r_X)</code>, posté sur le <code>flux_ecart</code> de X</li>
    </ul>

    <p><code>r_report</code> est le taux du **flux de report** du flux (la clôture où il se solde), résolu par compte via <code>v_flow_behavior</code>.</p>

    <h3>Types de taux</h3>
    <table className="help-table">
      <thead>
        <tr>
          <th>Valeur</th>
          <th>Source</th>
          <th>Utilisation</th>
        </tr>
      </thead>
      <tbody>
        <tr><td><code>close_n1</code></td><td>taux_ouverture N (clôture N-1 portée par N)</td><td>F00, F01</td></tr>
        <tr><td><code>avg</code></td><td>taux_moyen N</td><td>F20</td></tr>
        <tr><td><code>close_n</code></td><td>taux_close N</td><td>F80, F81, F98, F99</td></tr>
      </tbody>
    </table>

    <h3>Cas particuliers</h3>
    <table className="help-table">
      <thead>
        <tr>
          <th>Flux</th>
          <th>Taux</th>
          <th>Écart →</th>
        </tr>
      </thead>
      <tbody>
        <tr><td><strong>F00</strong> (ouverture)</td><td>close_n1</td><td>F80</td></tr>
        <tr><td><strong>F20</strong> (variation)</td><td>avg</td><td>F81</td></tr>
        <tr><td><strong>F99</strong> (clôture)</td><td>close_n</td><td>0</td></tr>
      </tbody>
    </table>

    <h2>4. Schémas de flux</h2>
    <p>Le comportement d'un flux dépend du compte : un compte de <strong>bilan</strong> applique le taux du flux avec écart F80/F81 et reporte sa clôture en ouverture N+1 ; un compte de <strong>résultat</strong> est au taux moyen sans écart et ne reporte pas.</p>

    <h3>Complétude</h3>
    <p>Un schéma doit définir **tous** les flux que portent ses comptes. Sinon, <code>v_flow_behavior</code> ne retourne pas de ligne de comportement → les écritures sont exclues de la conversion et de la reconstruction de clôture.</p>

    <h3>Schéma BILAN</h3>
    <p>Comptes de bilan : taux de conversion avec écarts (F00, F01 → close_n1/F80 ; F20 → avg/F81 ; F80, F81, F98, F99 → close_n). F99 s'auto-référence (flux_de_report = F99).</p>

    <h3>Schéma RESULTAT</h3>
    <p>Comptes de résultat : tous les flux au taux moyen sans écart (<code>flux_ecart = NULL</code>, <code>flux_a_nouveau = NULL</code>). F99 s'auto-référence (flux_de_report = F99).</p>

    <h2>5. Reconstruction des clôtures (F99)</h2>
    <p><strong>Identité de reconstruction</strong> :</p>
    <pre>
{`C = Σ(flux X | flux_de_report(X) = C et X ≠ C)`}
    </pre>
    <p>Un flux est une clôture reconstruite ssi <code>flux_de_report = flux</code> (auto-référence). Aujourd'hui seul F99 vérifie cette propriété.</p>

    <h3>Sémantique d'écrasement</h3>
    <p>La reconstruction est <strong>autoritaire</strong> : elle remplace toute valeur de clôture pré-existante pour un grain dimensionnel donné (DELETE ciblé puis INSERT).</p>

    <h3>Grain de reconstruction</h3>
    <p>Grain : toutes les dimensions propagées <strong>sauf <code>flow</code></strong> — la clôture est identifiée par <code>flux_de_report</code>. Cela inclut <strong>Nature</strong> et les dimensions analytiques (<code>partner</code>, <code>share</code>, <code>analysis</code>, <code>analysis2</code> + customs) : chaque « dont » obtient sa propre clôture.</p>

    <h3>Niveaux de reconstruction</h3>
    <p>La reconstruction est exécutée à chaque niveau de stockage (<code>corporate</code>, <code>converted</code>, <code>consolidated</code>) par <code>materialize_closures</code>.</p>

    <h2>6. À-nouveau</h2>
    <p>À la clôture, <strong>F99 (clôture N) se reporte sur F00 (ouverture N+1)</strong>. Le report est piloté par <code>flux_a_nouveau</code> (générique, jamais en dur).</p>

    <h3>Mécanisme</h3>
    <p>Le carry colle le solde de clôture C d'une consolidation N-1 figée (snapshot) sur le flux d'ouverture O du run courant, niveau par niveau :</p>
    <ul>
      <li><code>corporate</code> : F99 corporate du snapshot → F00 corporate (base de l'écart F80)</li>
      <li><code>consolidated</code> : F99 consolidé du snapshot → F00 consolidé (figé au % d'intégration N-1)</li>
    </ul>

    <p>Le converti ne fait pas de carry : la conversion native du F00 corporate reproduit le F99 converti N-1.</p>

    <h2>7. Catalogue des flux</h2>
    <table className="help-table">
      <thead>
        <tr>
          <th>Code</th>
          <th>Libellé</th>
          <th>Taux conversion</th>
          <th>Écart →</th>
          <th>Report →</th>
          <th>À-nouveau →</th>
          <th>Généré par</th>
        </tr>
      </thead>
      <tbody>
        <tr><td><strong>F00</strong></td><td>Ouverture</td><td>close_n1</td><td>F80</td><td>F99</td><td>NULL</td><td>Report d'ouverture (à-nouveau de F99 N-1)</td></tr>
        <tr><td><strong>F01</strong></td><td>Entrée de consolidation</td><td>close_n1</td><td>F80</td><td>F99</td><td>NULL</td><td>Variation de périmètre — entrée</td></tr>
        <tr><td><strong>F20</strong></td><td>Variation de bilan</td><td>avg</td><td>F81</td><td>F99</td><td>NULL</td><td>Saisie source / agrégation</td></tr>
        <tr><td><strong>F80</strong></td><td>Écart de conversion (ouverture → clôture)</td><td>close_n</td><td>NULL</td><td>F99</td><td>NULL</td><td>Conversion (écart de F00)</td></tr>
        <tr><td><strong>F81</strong></td><td>Conversion taux moyen → clôture</td><td>close_n</td><td>NULL</td><td>F99</td><td>NULL</td><td>Conversion (écart de F20)</td></tr>
        <tr><td><strong>F98</strong></td><td>Sortie de périmètre</td><td>close_n</td><td>NULL</td><td>F99</td><td>NULL</td><td>Variation de périmètre — sortie</td></tr>
        <tr><td><strong>F99</strong></td><td>Clôture</td><td>close_n</td><td>NULL</td><td>F99</td><td>F00</td><td>Reconstruction par identité</td></tr>
      </tbody>
    </table>

    <p><em>Sous le schéma RESULTAT, tous les flux passent au taux moyen sans écart (<code>flux_ecart = NULL</code>) et sans à-nouveau (<code>flux_a_nouveau = NULL</code>).</em></p>

    <h2>8. Traitements de consolidation par flux</h2>
    <h3>Entrée de périmètre → F01</h3>
    <p>Le F00 d'une entité entrante est déplacé vers F01 en consolidation. Ainsi le F00 consolidé ne contient que le report du périmètre existant.</p>

    <h3>Sortie de périmètre → F98</h3>
    <p>Une entité sortante garde ses flux constituants à l'identique, et chaque constituant X génère un miroir négatif <strong>−X sur F98</strong>. Ainsi <code>F99 = F00 + F20 + … + F98 = 0</code>.</p>

    <h3>Application des méthodes</h3>
    <ul>
      <li><strong>Intégration globale</strong> : agrégation des flux à 100%</li>
      <li><strong>Intégration proportionnelle</strong> : agrégation des flux au % d'intégration</li>
    </ul>

    <h2>9. API</h2>
    <ul>
      <li><code>GET/POST /api/flow-schemes</code> — CRUD des schémas de flux</li>
      <li><code>PUT/DELETE /api/flow-schemes/{`{code}`}</code></li>
      <li><code>GET/POST /api/flow-scheme-items</code> — CRUD des items de schéma</li>
    </ul>

    <h3>Voir aussi</h3>
    <ul>
      <li><NavLink to='regles'>Règles (utilisent les flux)</NavLink></li>
      <li><NavLink to='taux-integration'>Taux d'intégration (application des méthodes)</NavLink></li>
    </ul>
  </div>
);

const aNouveauContent = (
  <div className="help-content">
    <h2>Vue d'ensemble</h2>
    <p>
      L'<strong>à-nouveau</strong> est le mécanisme de report d'ouverture entre exercices : le solde de clôture (flux F99) d'une consolidation N-1 figée (snapshot) est reporté sur l'ouverture (flux F00) de la consolidation N. Ce report garantit la <strong>continuité du bilan consolidé</strong> d'un exercice à l'autre.
    </p>

    <h2>1. Définition</h2>
    <p>
      L'à-nouveau est le report automatique des soldes de clôture de l'exercice N-1 vers l'ouverture de l'exercice N. Le moteur ne code jamais en dur <code>F00</code> ni <code>F99</code> : le rôle « ouverture issue d'un à-nouveau » est déclaré par une donnée <code>flux_a_nouveau</code> dans le schéma de flux (<code>sat_flow_scheme_item</code>), résolu par compte via la vue <code>v_flow_behavior</code>.
    </p>

    <p>Aujourd'hui, seul le couple <strong>F99 → F00</strong> est déclaré en à-nouveau. Le résultat ne reporte pas d'à-nouveau (son schéma a <code>flux_a_nouveau = NULL</code>), seul le bilan le fait.</p>

    <h2>2. Mécanique moteur</h2>
    <h3>2.1 Snapshot figé</h3>
    <p>
      Le report lit le F99 stocké d'un run N-1 <strong>déjà calculé et verrouillé</strong>, par niveau de stockage. Le statut « ouvert » est toléré avec un avertissement, mais l'idéal est un snapshot figé.
    </p>

    <h3>2.2 Injection du report</h3>
    <p>À l'ouverture du run N, pour chaque flux source d'à-nouveau C (= F99) et son ouverture cible O (= F00) :</p>
    <ol>
      <li><strong>Écrase</strong> le flux d'ouverture O du run courant (DELETE ciblé) — le F00 issu de la liasse est remplacé</li>
      <li><strong>Colle</strong> le solde de clôture C du snapshot (INSERT), relabellisé en O, phase/période repointés sur le run N</li>
    </ol>

    <pre>
      {`F00[N, corporate]  ←  F99[snapshot N-1, corporate]      (écrase le F00 de liasse)
      F00[N, consolidé]  ←  F99[snapshot N-1, consolidé]`}
    </pre>

    <p>Ce montant corporate est <strong>autoritaire</strong> et sert de base à tout le reste (écarts de conversion, report sur la clôture).</p>

    <h2>3. Niveaux de carry</h2>
    <p>Le report opère à <strong>deux niveaux</strong> uniquement :</p>

    <table className="help-table">
      <thead>
        <tr>
          <th>Niveau</th>
          <th>Mécanique</th>
          <th>Justification</th>
        </tr>
      </thead>
      <tbody>
        <tr>
          <td><strong>Corporate</strong></td>
          <td>Colle le F99 corporate du snapshot → F00 corporate (écrase la liasse)</td>
          <td>Base de l'écart F80 (revalorisation au taux de clôture N)</td>
        </tr>
        <tr>
          <td><strong>Converti</strong></td>
          <td><strong>Pas de carry</strong></td>
          <td>La conversion native du F00 corporate reproduit le F99 converti N-1 (via taux close_n1)</td>
        </tr>
        <tr>
          <td><strong>Consolidé</strong></td>
          <td>Colle le F99 consolidé du snapshot → F00 consolidé</td>
          <td>Figé au % d'intégration N-1 (la variation vers le % N est une règle)</td>
        </tr>
      </tbody>
    </table>

    <h2>4. Périmètre du report</h2>
    <h3>4.1 Entités consolidées en N-1 seulement</h3>
    <p>
      Le carry ne concerne que les entités <strong>effectivement consolidées</strong> dans le snapshot, c.-à-d. celles qui y portent une clôture F99 au niveau <code>consolidated</code>. Les entités absentes (nouvelles entrées) gardent leur F00 de liasse (reclassé en F01 par règle).
    </p>

    <p><strong>Comment savoir ?</strong> Une entité était consolidée en N-1 ssi le snapshot porte une clôture consolidée pour elle :</p>
    <pre>
      {`consolidée_en_N1(E)  ⇔  EXISTS ( fact_entry
                                       WHERE consolidation_id = <snapshot>
                                         AND entity   = E
                                         AND level    = 'consolidated'
                                         AND flow     = 'F99' )`}
    </pre>

    <h3>4.2 Entités dans le scope du run courant</h3>
    <p>
      Le carry ne s'applique qu'aux entités présentes dans le périmètre du run N (<code>sat_perimeter</code>). Une entité consolidée en N-1 mais sortie du périmètre N ne reçoit pas de report (son solde reste dans le snapshot).
    </p>

    <h2>5. Utilisation des taux N-1</h2>
    <h3>5.1 % d'intégration figé</h3>
    <p>
      Le F00 consolidé est figé au <strong>% d'intégration N-1</strong> (collecté depuis le snapshot). L'étape D applique le <code>× pct_integration</code> sur tous les flux <strong>sauf F00</strong> (exemption data-driven via <code>v_flow_behavior</code>). Le F00 consolidé reste donc au % N-1.
    </p>

    <h3>5.2 Variation de % d'intégration</h3>
    <p>
      La variation vers le % N n'est <strong>pas native</strong> : elle est gérée par une <strong>règle</strong> au niveau converti. La règle calcule et poste la différence :
    </p>
    <pre>
      {`<flux de variation>  =  F00_converti × (pct_integration_N − pct_integration_{N-1})`}
    </pre>
    <p>Ainsi, <code>F00 + {`<flux de variation>`}</code> au consolidé = <code>F00_converti × pct_N</code>.</p>

    <h2>6. Écritures automatiques et exemptions</h2>
    <h3>6.1 F00 exempté des transforms natives</h3>
    <p>
      Une fois le F00 collé à chaque niveau, les étapes natives qui *produisent* ces niveaux ne doivent <strong>pas recalculer</strong> sa valeur (sinon double compte). Règle générale data-driven : la branche « value-producing » de chaque étape exclut les flux cibles d'à-nouveau.
    </p>

    <table className="help-table">
      <thead>
        <tr>
          <th>Étape</th>
          <th>Effet sur F00</th>
        </tr>
      </thead>
      <tbody>
        <tr>
          <td><strong>Conversion</strong></td>
          <td><strong>Aucune exemption</strong> : la conversion s'applique normalement au F00 corporate. Elle produit le montant converti <code>F00_corporate × taux_clôture_{`{N-1}`}</code> <strong>et</strong> l'écart F80 = <code>F00_corporate × (taux_clôture_{`{N}`} − taux_clôture_{`{N-1}`})</code></td>
        </tr>
        <tr>
          <td><strong>Consolidation</strong></td>
          <td><code>× pct_integration</code> sur <strong>tous les flux sauf F00</strong> : le F00 consolidé est collé du snapshot (figé au % N-1), l'étape D ne le re-multiplie pas</td>
        </tr>
        <tr>
          <td><strong>Reconstruction clôture</strong></td>
          <td>F00 reporte à F99 normalement (<code>flux_de_report</code>), depuis le montant corporate. L'identité <code>F99 = F00 + Σ variations + Σ écarts</code> se referme à chaque niveau</td>
        </tr>
      </tbody>
    </table>

    <h2>7. Configuration</h2>
    <h3>7.1 dim_consolidation.a_nouveau_consolidation_id</h3>
    <p>
      Chaque consolidation référence <strong>facultativement</strong> la consolidation d'à-nouveau via <code>a_nouveau_consolidation_id</code> (FK vers <code>dim_consolidation.id</code>). <code>NULL</code> = pas d'à-nouveau.
    </p>

    <h3>7.2 Schéma de flux</h3>
    <p>Le champ <code>flux_a_nouveau</code> de <code>sat_flow_scheme_item</code> déclare pour chaque flux C (clôture) le flux O (ouverture) qui reçoit son solde à l'exercice suivant :</p>
    <pre>
      {`sat_flow_scheme_item
      ├── scheme          : FK dim_flow_scheme.id
      ├── flow            : code du flux (ex. 'F99')
      ├── flux_a_nouveau  : code du flux d'ouverture cible (ex. 'F00'), NULL sinon`}
    </pre>

    <h3>7.3 Vue v_flow_behavior</h3>
    <p>
      La résolution <code>compte → schéma → comportement</code> passe par la vue <code>v_flow_behavior</code>, consommée par <code>pipeline::a_nouveau</code> pour lire les couples (source, target) d'à-nouveau.
    </p>

    <h2>8. Exemples concrets</h2>
    <h3>8.1 Filiale qui change de % d'intégration</h3>
    <p>Une filiale F contrôlée à 70% en N-1, 80% en N :</p>
    <ul>
      <li><strong>Situation N-1</strong> : F99 consolidé = 700 (capital 1000 × 0.70)</li>
      <li><strong>Report N</strong> : F00 consolidé = 700 (figé au % N-1)</li>
      <li><strong>Règle de variation</strong> : F90 = 1000 × (0.80 − 0.70) = 100</li>
      <li><strong>Bilan N</strong> : F00 + F90 = 700 + 100 = 800 (= 1000 × 0.80)</li>
    </ul>

    <h3>8.2 Entité entrante</h3>
    <p>Une entité E qui n'était pas consolidée en N-1 entre dans le périmètre en N :</p>
    <ul>
      <li><strong>Pas de report</strong> : E n'a pas de F99 consolidé dans le snapshot N-1</li>
      <li><strong>F00 de liasse</strong> : E remonte son F00 depuis la liasse (ex. 500)</li>
      <li><strong>Règle d'entrée</strong> : reclasse F00→F01 (entrée de consolidation)</li>
      <li><strong>F00 consolidé</strong> : ne contient que le report du périmètre existant</li>
    </ul>

    <h3>8.3 Cas sans à-nouveau (première consolidation)</h3>
    <p>Si <code>a_nouveau_consolidation_id = NULL</code> :</p>
    <ul>
      <li>Aucune entité n'a de report → toutes sont traitées comme entrantes</li>
      <li>Tous les F00 sont reclassés sur F01 par règle</li>
      <li>Le périmètre N doit refléter cela (<code>entree = true</code> pour toutes les entités)</li>
    </ul>

    <h2>9. Traitements de périmètre (devenus règles)</h2>
    <p>
      Les traitements de périmètre (F00→F01 pour les entrants, miroir −X sur F98 pour les sortants) sont <strong>devenus des règles</strong> au niveau corporate. Ils ne sont plus natifs (l'étape B/reclassification a été supprimée).
    </p>

    <table className="help-table">
      <thead>
        <tr>
          <th>Traitement</th>
          <th>Niveau</th>
          <th>Mécanique</th>
        </tr>
      </thead>
      <tbody>
        <tr>
          <td><strong>Entrée de périmètre</strong></td>
          <td>Corporate</td>
          <td>Règle : reclasser le F00 des entités entrantes vers F01 (entree = true)</td>
        </tr>
        <tr>
          <td><strong>Sortie de périmètre</strong></td>
          <td>Corporate</td>
          <td>Règle : chaque constituant X génère −X sur F98 (sortie = true)</td>
        </tr>
        <tr>
          <td><strong>Variation de % d'intégration</strong></td>
          <td>Converti</td>
          <td>Règle : calculer et poster F90/F95 = F00 × (pct_N − pct_N-1)</td>
        </tr>
      </tbody>
    </table>

    <h2>10. API</h2>
    <ul>
      <li><code>GET/POST /api/consolidations</code> — CRUD des consolidations (champ <code>a_nouveau_consolidation_id</code>)</li>
      <li><code>PUT /api/consolidations/{`{id}`}</code> — modifier la consolidation d'à-nouveau</li>
      <li><code>POST /api/run</code> — exécute le pipeline avec à-nouveau si configuré (renvoie <code>a_nouveau_warnings</code>)</li>
    </ul>

    <h3>Types TypeScript</h3>
    <pre>
      {`interface Consolidation {
        // ...
        a_nouveau_consolidation_id: number | null;  // FK vers dim_consolidation
        // ...
      }

      interface PipelineRunResult {
        // ...
        a_nouveau_warnings: CoherenceWarning[];  // alertes de cohérence
      }`}
    </pre>

    <h3>Voir aussi</h3>
    <ul>
      <li><NavLink to='taux-integration'>Taux d'intégration natif (exemption F00, variation de %)</NavLink></li>
      <li><NavLink to='schemas-flux'>Schémas de flux (flux_a_nouveau, v_flow_behavior)</NavLink></li>
      <li><NavLink to='regles'>Règles (traitements de périmètre F00→F01, F98, F90/F95)</NavLink></li>
    </ul>
  </div>
);

const perimetresContent = (
  <div className="help-content">
    <h2>Vue d'ensemble</h2>
    <p>
      Les <strong>Jeux de périmètre</strong> définissent les entités qui participent à une consolidation et leurs caractéristiques d'intégration. Un jeu de périmètre est une <strong>version du périmètre</strong> (réel, budget, prévision) référencée par une consolidation via <code>dim_consolidation.perimeter_set</code>. Le périmètre est versionné comme les taux de change, permettant sa réutilisation entre consolidations et variantes.
    </p>

    <h2>1. Définition d'un jeu de périmètre</h2>
    <h3>Tables impliquées</h3>
    <ul>
      <li><code>dim_perimeter_set</code> : catalogue des jeux de périmètre (code, libellé)</li>
      <li><code>sat_perimeter</code> : composition du périmètre par (perimeter_set, entity, period)</li>
      <li><code>dim_method</code> : méthodes de consolidation (globale, proportionnelle, équivalence…)</li>
    </ul>

    <h3>Clé du périmètre</h3>
    <p>La table <code>sat_perimeter</code> est clé par <code>(perimeter_set, entity, period)</code>. Chaque combinaison définit comment une entité est intégrée pour une période donnée d'un jeu de périmètre.</p>

    <h2>2. Structure de sat_perimeter</h2>
    <p>Chaque ligne de <code>sat_perimeter</code> définit les caractéristiques d'intégration d'une entité :</p>
    <table className="help-table">
      <thead>
        <tr>
          <th>Champ</th>
          <th>Type</th>
          <th>Rôle</th>
        </tr>
      </thead>
      <tbody>
        <tr><td><code>perimeter_set</code></td><td>INTEGER</td><td>Clé du jeu de périmètre (FK dim_perimeter_set.id)</td></tr>
        <tr><td><code>entity</code></td><td>TEXT</td><td>Code de l'entité (FK dim_entity.code)</td></tr>
        <tr><td><code>period</code></td><td>TEXT</td><td>Période (correspond à Entry_period)</td></tr>
        <tr><td><code>methode</code></td><td>INTEGER</td><td>Méthode de consolidation (FK dim_method.id)</td></tr>
        <tr><td><code>pct_interet</code></td><td>DECIMAL(10,4)</td><td>Pourcentage d'intérêt (détention financière)</td></tr>
        <tr><td><code>pct_integration</code></td><td>DECIMAL(10,4)</td><td>Pourcentage d'intégration (contrôle, ex. 1.0 pour la globale)</td></tr>
        <tr><td><code>entree</code></td><td>BOOLEAN</td><td>Entité entrante dans le périmètre (défaut = FALSE)</td></tr>
        <tr><td><code>sortie</code></td><td>BOOLEAN</td><td>Entité sortante du périmètre (défaut = FALSE)</td></tr>
      </tbody>
    </table>

    <h2>3. Méthodes de consolidation</h2>
    <p>La table <code>dim_method</code> définit les méthodes disponibles avec un flag <code>consolidated</code> :</p>
    <table className="help-table">
      <thead>
        <tr>
          <th>Méthode</th>
          <th>consolidated</th>
          <th>Comportement</th>
        </tr>
      </thead>
      <tbody>
        <tr><td><strong>Globale</strong></td><td>true</td><td>Agrégation à 100% (pct_integration = 1.0)</td></tr>
        <tr><td><strong>Proportionnelle</strong></td><td>true</td><td>Agrégation au % d'intégration (pct_integration ∈ ]0, 1[)</td></tr>
        <tr><td><strong>Équivalence</strong></td><td>false</td><td>Exclue du niveau consolidated (post-MVP, via règles)</td></tr>
      </tbody>
    </table>

    <p>L'étape C du pipeline filtre par <code>JOIN dim_method m ON m.code = p.methode WHERE m.consolidated</code> : seules les méthodes consolidées passent au niveau <code>consolidated</code>.</p>

    <h2>4. Perspectives de lecture du périmètre</h2>
    <p>Les coefficients et les règles peuvent lire le périmètre à <strong>quatre perspectives</strong> différentes :</p>
    <table className="help-table">
      <thead>
        <tr>
          <th>Perspective</th>
          <th>Entité lue</th>
          <th>Période</th>
        </tr>
      </thead>
      <tbody>
        <tr><td><code>entity</code></td><td>l'entité de l'écriture</td><td>courante</td></tr>
        <tr><td><code>partner</code></td><td>le partenaire de l'écriture</td><td>courante</td></tr>
        <tr><td><code>entity_n1</code></td><td>l'entité</td><td>N-1 (via à-nouveau)</td></tr>
        <tr><td><code>partner_n1</code></td><td>le partenaire</td><td>N-1 (via à-nouveau)</td></tr>
      </tbody>
    </table>

    <p>Champs accessibles par perspective : <code>pct_integration</code>, <code>pct_interet</code> (et tout champ numérique de <code>sat_perimeter</code> whitelisté).</p>

    <h3>Utilisation dans les coefficients</h3>
    <p>Les coefficients nommés peuvent lire ces perspectives via la syntaxe <code>[champ.perspective]</code> :</p>
    <pre>
{`[pct_integration.entity]   → % d'intégration de l'entité courante
[pct_integration.partner]   → % d'intégration du partenaire
[pct_integration.entity_n1] → % d'intégration de l'entité en N-1
[pct_interet.partner]       → % d'intérêt du partenaire`}
    </pre>

    <h2>5. À-nouveau et périmètre N-1</h2>
    <p>L'à-nouveau utilise le périmètre pour identifier les entités éligibles au carry (report d'ouverture) :</p>

    <h3>Entités éligibles</h3>
    <p>Le carry ne concerne que les entités <strong>effectivement consolidées</strong> dans le snapshot N-1 :</p>
    <ul>
      <li>Présence d'une clôture F99 au niveau <code>consolidated</code> dans le snapshot</li>
      <li>Présence dans le <code>sat_perimeter</code> du run courant</li>
    </ul>

    <h3>Filtrage</h3>
    <p>Le carry filtre par <code>entity IN (SELECT entity FROM sat_perimeter WHERE perimeter_set = ? AND period = ?)</code>. Les entités absentes du périmètre courant gardent leur F00 de liasse (reclassé en F01 par règle).</p>

    <h3>Conséquence</h3>
    <p>Le F00 consolidé est <strong>figé au % d'intégration N-1</strong> (collecté depuis le snapshot). La variation vers le % N est une règle (post-consolidation), pas un traitement natif.</p>

    <h2>6. Interface utilisateur</h2>
    <h3>Page Périmètres</h3>
    <p>L'interface utilise un composant <strong>Master-Detail</strong> avec :</p>
    <ul>
      <li><strong>Object key</strong> : jeu de périmètre + période (dropdowns)</li>
      <li><strong>Grille</strong> : liste des entités avec leurs caractéristiques</li>
    </ul>

    <h3>Colonnes éditables</h3>
    <ul>
      <li><code>entity</code> : dropdown depuis les entités</li>
      <li><code>methode</code> : dropdown depuis les méthodes</li>
      <li><code>pct_interet</code> : nombre (0 à 1)</li>
      <li><code>pct_integration</code> : nombre (0 à 1)</li>
      <li><code>entree</code> : booléen (case à cocher)</li>
      <li><code>sortie</code> : booléen (case à cocher)</li>
    </ul>

    <h3>Règles d'édition</h3>
    <ul>
      <li>La clé <code>(perimeter_set, entity, period)</code> est unique — pas de doublons</li>
      <li>Une entité ne peut être à la fois entrante et sortante pour la même période</li>
      <li><code>pct_integration</code> doit être ≤ <code>pct_interet</code> (validation côté serveur)</li>
    </ul>

    <h2>7. API REST</h2>
    <ul>
      <li><code>GET/POST /api/md/perimeter_sets</code> — CRUD des jeux de périmètre</li>
      <li><code>GET/POST /api/md/perimeter</code> — CRUD du périmètre (sat_perimeter)</li>
      <li><code>POST /api/import/perimeter</code> — import CSV du périmètre</li>
    </ul>

    <h3>Format d'import CSV</h3>
    <pre>
{`perimeter_set,entity,period,methode,pct_interet,pct_integration,entree,sortie
PERIM_REEL,A,2024,globale,1.0,1.0,false,false
PERIM_REEL,B,2024,proportionnelle,0.8,0.8,false,false
PERIM_REEL,C,2024,equivalence,0.3,0.3,true,false`}
    </pre>

    <h2>8. Exemples concrets</h2>
    <h3>Exemple 1 : Filiale contrôlée à 80%</h3>
    <p>Une filiale F détenue à 80% en méthode globale :</p>
    <pre>
{`perimeter_set  | entity | period | methode      | pct_interet | pct_integration | entree | sortie
PERIM_REEL      | F      | 2024   | globale      | 0.8000      | 1.0000          | false  | false`}
    </pre>
    <p>Comportement :</p>
    <ul>
      <li><strong>Corporate</strong> : F00 = 1000, F20 = 200 (montants bruts)</li>
      <li><strong>Consolidé</strong> : F00 = 1000×1.0_N-1 (reporté), F20 = 200×1.0 = 200 (appliqué nativement)</li>
      <li><strong>Intérêts minoritaires</strong> : calculés par règle = <code>(0.8 - 1.0) × montant</code></li>
    </ul>

    <h3>Exemple 2 : Filiale en équivalence (post-MVP)</h3>
    <p>Une filiale E détenue à 30% en méthode équivalence :</p>
    <pre>
{`perimeter_set  | entity | period | methode     | pct_interet | pct_integration | entree | sortie
PERIM_REEL      | E      | 2024   | equivalence | 0.3000      | 0.3000          | false  | false`}
    </pre>
    <p>Comportement :</p>
    <ul>
      <li><strong>Consolidé</strong> : aucune écriture (exclue par <code>consolidated = false</code>)</li>
      <li><strong>Représentation</strong> : règle post-consolidation qui quote-part le résultat net sur un compte d'équivalence</li>
    </ul>

    <h3>Exemple 3 : Entité entrante</h3>
    <p>Une entité acquise en cours d'exercice :</p>
    <pre>
{`perimeter_set  | entity | period | methode | pct_interet | pct_integration | entree | sortie
PERIM_REEL      | A      | 2024   | globale | 1.0000      | 1.0000          | true   | false`}
    </pre>
    <p>Comportement :</p>
    <ul>
      <li><strong>Corporate</strong> : F00 = 500 (issu de la liasse)</li>
      <li><strong>Règle corporate</strong> : reclasse F00 → F01 (flux d'entrée de périmètre)</li>
      <li><strong>Converti</strong> : F01 converti au taux close_n1</li>
      <li><strong>Consolidé</strong> : F01 × 1.0 = entrant consolidé</li>
    </ul>

    <h3>Exemple 4 : Entité sortante</h3>
    <p>Une entité cédée en cours d'exercice :</p>
    <pre>
{`perimeter_set  | entity | period | methode | pct_interet | pct_integration | entree | sortie
PERIM_REEL      | B      | 2024   | globale | 0.7000      | 1.0000          | false  | true`}
    </pre>
    <p>Comportement :</p>
    <ul>
      <li><strong>Avant cession</strong> : flux normaux (F00, F20, etc.)</li>
      <li><strong>À la cession</strong> : chaque flux constituant X génère un miroir négatif <strong>−X sur F98</strong> (sortie de périmètre)</li>
      <li><strong>Consolidé</strong> : <code>F99 = F00 + F20 + … + F98 = 0</code> (l'entité disparaît du bilan consolidé)</li>
    </ul>

    <h3>Voir aussi</h3>
    <ul>
      <li><NavLink to='regles'>Utilisation du périmètre dans les règles (scope, coefficients)</NavLink></li>
      <li><NavLink to='coefficients'>Perspectives entity/partner/entity_n1/partner_n1</NavLink></li>
      <li><NavLink to='taux-integration'>Application du % d'intégration natif</NavLink></li>
      <li><NavLink to='schemas-flux'>Flux d'entrée/sortie (F01, F98)</NavLink></li>
    </ul>
  </div>
);

const tauxIntegrationContent = (
  <div className="help-content">
    <h2>Vue d'ensemble</h2>
    <p>
      L'<strong>application du taux d'intégration natif</strong> est le mécanisme par lequel le moteur applique le pourcentage de contrôle des entités lors de l'étape de consolidation (étape C du pipeline). Ce traitement est <strong>automatique et natif</strong> : il s'applique à toutes les écritures du niveau <code>corporate</code> pour produire le niveau <code>consolidated</code>, selon la méthode d'intégration définie dans le périmètre.
    </p>

    <h2>1. Application du taux d'intégration</h2>
    <h3>Principe de base</h3>
    <p>À l'étape C du pipeline, chaque écriture du niveau <code>converted</code> est multipliée par son <strong>pourcentage d'intégration</strong> (<code>pct_integration</code>) :</p>
    <pre>
{`amount_consolidated = amount_converted × pct_integration`}
    </pre>
    <p>Le <code>pct_integration</code> provient de la table <code>sat_perimeter</code> pour l'entité concernée.</p>

    <h3>Flux exemptés</h3>
    <p>Le <strong>F00 (ouverture)</strong> est <strong>exempté</strong> du <code>× pct_integration</code> à l'étape de consolidation. Concrètement :</p>
    <ul>
      <li><strong>Corporate</strong> : le F00 corporate provient de la liasse (sauf si reporté par à-nouveau)</li>
      <li><strong>Converti</strong> : le F00 converti est obtenu par conversion normale (taux close_n1)</li>
      <li><strong>Consolidé</strong> : le F00 consolidé est <strong>collecté depuis le snapshot N-1</strong> (figé au % d'intégration N-1) et <strong>non re-multiplié</strong> par le % courant</li>
    </ul>
    <p>Tous les autres flux (F01, F20, F80, F81, F98, F99, etc.) subissent le <code>× pct_integration</code>.</p>

    <h3>Méthodes de consolidation</h3>
    <p>La table <code>dim_method</code> définit les méthodes de consolidation avec un flag <code>consolidated</code> :</p>

    <table className="help-table">
      <thead>
        <tr>
          <th>Méthode</th>
          <th>consolidated</th>
          <th>Comportement</th>
        </tr>
      </thead>
      <tbody>
        <tr><td><strong>Globale</strong></td><td>true</td><td>pct_integration = 1.0 (100%)</td></tr>
        <tr><td><strong>Proportionnelle</strong></td><td>true</td><td>pct_integration ∈ ]0, 1[</td></tr>
        <tr><td><strong>Équivalence</strong></td><td>false</td><td>Exclue du niveau consolidated (post-MVP)</td></tr>
      </tbody>
    </table>

    <p>L'étape C filtre par <code>JOIN dim_method m ON m.code = p.methode WHERE m.consolidated</code> : seules les méthodes consolidées passent au niveau <code>consolidated</code>.</p>

    <h3>Grain d'application</h3>
    <p>Le taux d'intégration s'applique au <strong>grain de l'entité</strong>. Pour chaque écriture au niveau <code>converted</code> :</p>
    <ul>
      <li>Jointure sur <code>sat_perimeter</code> via <code>entity</code> et <code>entry_period</code></li>
      <li>Application de <code>pct_integration</code> depuis <code>sat_perimeter</code></li>
      <li>Génération d'une écriture au niveau <code>consolidated</code> avec le montant pondéré</li>
    </ul>

    <h2>2. Comportement des règles</h2>
    <h3>Superposition au natif</h3>
    <p>Les règles de consolidation peuvent <strong>ajouter des comportements</strong> par-dessus le traitement natif. Elles opèrent à différents niveaux :</p>
    <ul>
      <li><strong>Corporate</strong> : règles de périmètre (F00→F01 entrants, miroir F98 sortants)</li>
      <li><strong>Converti</strong> : règles de variation de % (F90/F95), éliminations interco</li>
      <li><strong>Consolidé</strong> : règles post-consolidation (intérêts minoritaires, retraitements)</li>
    </ul>

    <h3>Coefficients de taux</h3>
    <p>Les règles peuvent utiliser les coefficients nommés de la bibliothèque <code>dim_coefficient</code> :</p>
    <table className="help-table">
      <thead>
        <tr>
          <th>Code</th>
          <th>Expression</th>
          <th>Description</th>
        </tr>
      </thead>
      <tbody>
        <tr><td><code>pct_integration</code></td><td><code>[pct_integration.entity]</code></td><td>Pourcentage d'intégration de l'entité</td></tr>
        <tr><td><code>pct_interet</code></td><td><code>[pct_interet.entity]</code></td><td>Pourcentage d'intérêt de l'entité</td></tr>
      </tbody>
    </table>
    <p>Ces coefficients lisent <code>sat_perimeter.pct_integration</code> ou <code>pct_interet</code> pour l'entité (ou le partenaire) concernée.</p>

    <h3>Exemples de comportements</h3>
    <ul>
      <li><strong>Variation de périmètre</strong> : règle au niveau corporate qui reclasse le F00 des entités entrantes vers F01</li>
      <li><strong>Éliminations interco</strong> : règle au niveau converti qui extourne les soldes interco au prorata du plus faible taux d'intégration</li>
      <li><strong>Intérêts minoritaires</strong> : règle au niveau consolidé qui calcule la quote-part non contrôlée (<code>pct_interet - pct_integration</code>)</li>
      <li><strong>Retraitements</strong> : règle au niveau consolidé qui réaffecte des montants selon des règles comptables spécifiques</li>
    </ul>

    <h2>3. Variation de taux d'intégration et roll-forward</h2>
    <h3>Pas de traitement natif</h3>
    <p>La <strong>variation de taux d'intégration</strong> (ex : passage de 60% à 80% d'une année sur l'autre) n'est <strong>pas traitée nativement</strong> par le moteur. Ce comportement est délibérément laissé aux règles.</p>

    <h3>Mécanique via règles</h3>
    <p>Le F00 consolidé est <strong>figé au % N-1</strong> (collecté depuis le snapshot d'à-nouveau). Pour l'aligner sur le % N :</p>
    <ol>
      <li>Le <strong>carry (à-nouveau)</strong> colle le F99 consolidé N-1 sur le F00 consolidé N, figé au % N-1</li>
      <li>Une <strong>règle de variation</strong> au niveau converti calcule et poste la différence</li>
    </ol>
    <pre>
{`variation = F00_converti × (pct_integration_N − pct_integration_N-1)
F90/F95 = variation`}
    </pre>
    <p>Ainsi, <code>F00 + F90/F95</code> au consolidé = <code>F00_converti × pct_N</code>.</p>

    <h3>Importance pour les roll-forward</h3>
    <p>Gérer la variation via les règles garantit des <strong>roll-forward propres</strong> :</p>
    <ul>
      <li>L'ouverture consolidée reste <strong>traçable</strong> (F00 = report exact N-1)</li>
      <li>La variation est <strong>explicitée</strong> dans un flux dédié (F90/F95 ou autre)</li>
      <li>Le bilan de clôture reste <strong>cohérent</strong> (F99 = F00 + Σ variations)</li>
    </ul>

    <h2>4. Tables et données impliquées</h2>
    <h3>sat_perimeter</h3>
    <p>Table satellite contenant la définition du périmètre de consolidation par entité :</p>
    <table className="help-table">
      <thead>
        <tr>
          <th>Champ</th>
          <th>Type</th>
          <th>Rôle</th>
        </tr>
      </thead>
      <tbody>
        <tr><td><code>perimeter_set</code></td><td>INTEGER</td><td>Clé du jeu de périmètre (FK dim_perimeter_set.id)</td></tr>
        <tr><td><code>entity</code></td><td>TEXT</td><td>Code de l'entité</td></tr>
        <tr><td><code>period</code></td><td>TEXT</td><td>Période (Entry_period)</td></tr>
        <tr><td><code>methode</code></td><td>INTEGER</td><td>Méthode de consolidation (FK dim_method.id)</td></tr>
        <tr><td><code>pct_interet</code></td><td>DECIMAL(10,4)</td><td>Pourcentage d'intérêt (détention)</td></tr>
        <tr><td><code>pct_integration</code></td><td>DECIMAL(10,4)</td><td>Pourcentage d'intégration (contrôle)</td></tr>
        <tr><td><code>entree</code></td><td>BOOLEAN</td><td>Entité entrante dans le périmètre</td></tr>
        <tr><td><code>sortie</code></td><td>BOOLEAN</td><td>Entité sortante du périmètre</td></tr>
      </tbody>
    </table>

    <h3>fact_entry</h3>
    <p>Table de faits contenant les écritures aux 3 niveaux :</p>
    <table className="help-table">
      <thead>
        <tr>
          <th>Champ</th>
          <th>Type</th>
          <th>Rôle</th>
        </tr>
      </thead>
      <tbody>
        <tr><td><code>level</code></td><td>TEXT</td><td>Niveau de stockage (<code>corporate</code>, <code>converted</code>, <code>consolidated</code>)</td></tr>
        <tr><td><code>entity</code></td><td>INTEGER</td><td>Dimension Entity (id technique)</td></tr>
        <tr><td><code>amount</code></td><td>DECIMAL(18,2)</td><td>Montant de l'écriture</td></tr>
      </tbody>
    </table>
    <p>Le niveau <code>consolidated</code> contient les écritures après application du <code>× pct_integration</code>.</p>

    <h3>dim_method</h3>
    <p>Table de référence des méthodes de consolidation :</p>
    <table className="help-table">
      <thead>
        <tr>
          <th>Champ</th>
          <th>Type</th>
          <th>Rôle</th>
        </tr>
      </thead>
      <tbody>
        <tr><td><code>code</code></td><td>TEXT</td><td>Code de la méthode</td></tr>
        <tr><td><code>libelle</code></td><td>TEXT</td><td>Libellé de la méthode</td></tr>
        <tr><td><code>consolidated</code></td><td>BOOLEAN</td><td>Flag : la méthode est-elle intégrée au niveau consolidated ?</td></tr>
      </tbody>
    </table>

    <h2>5. Exemples concrets</h2>
    <h3>Exemple 1 : Filiale contrôlée à 80%</h3>
    <p>Une filiale F avec <code>pct_integration = 0.80</code> en méthode globale :</p>
    <ul>
      <li><strong>Converti</strong> : F00 = 1000, F20 = 200</li>
      <li><strong>Consolidé</strong> : F00 (reporté) = 1000×0.80_N-1, F20 = 200×0.80 = 160</li>
      <li><strong>Si le % passe de 70% (N-1) à 80% (N)</strong> : règle de variation poste F90 = 1000×(0.80−0.70) = 100</li>
    </ul>

    <h3>Exemple 2 : Filiale en équivalence (post-MVP)</h3>
    <p>Une filiale E en méthode équivalence (<code>consolidated = false</code>) :</p>
    <ul>
      <li><strong>Converti</strong> : F00 = 1000, F20 = 200</li>
      <li><strong>Consolidé</strong> : aucune écriture (exclue par le filtre <code>m.consolidated</code>)</li>
      <li><strong>Représentation</strong> : par une règle post-consolidation qui quote-part le résultat net</li>
    </ul>

    <h3>Exemple 3 : Entité entrante</h3>
    <p>Une entité entrante E avec <code>entree = true</code> :</p>
    <ul>
      <li><strong>Corporate</strong> : F00 = 500 (issu de la liasse)</li>
      <li><strong>Règle corporate</strong> : reclasse F00→F01 (entrants)</li>
      <li><strong>Converti</strong> : F01 converti au taux close_n1</li>
      <li><strong>Consolidé</strong> : F01×pct_integration = entrant consolidé</li>
    </ul>

    <h3>Voir aussi</h3>
    <ul>
      <li><NavLink to='regles'>Variation de taux via règles</NavLink></li>
      <li><NavLink to='schemas-flux'>Exemption du flux F00 (à-nouveau)</NavLink></li>
    </ul>
  </div>
);

const dimensionsMasterdataContent = (
  <div className="help-content">
    <h2>Vue d'ensemble</h2>
    <p>
      Les <strong>Dimensions</strong> sont les axes d'analyse de la table de faits <code>fact_entry</code>. Chaque dimension correspond à une colonne de la table de faits qui est propagée à travers le pipeline de consolidation (agrégation, conversion, consolidation). Le <strong>Master Data</strong> regroupe toutes les tables de référence qui alimentent ces dimensions (entités, comptes, périodes, flux, etc.).
    </p>

    <h2>1. Qu'est-ce qu'une dimension ?</h2>
    <p>Une dimension est une <strong>colonne de la table de faits</strong> qui porte une valeur sémantique (ex: <code>entity</code>, <code>account</code>, <code>flow</code>). Elle est <strong>propagée</strong> à travers les 3 niveaux de stockage (<code>corporate</code>, <code>converted</code>, <code>consolidated</code>).</p>

    <h3>Built-in vs Custom</h3>
    <ul>
      <li><strong>Built-in</strong> : 12 dimensions prédéfinies par le moteur (<code>phase</code>, <code>entity</code>, <code>entry_period</code>, <code>period</code>, <code>account</code>, <code>flow</code>, <code>currency</code>, <code>nature</code>, <code>partner</code>, <code>share</code>, <code>analysis</code>, <code>analysis2</code>). Elles sont verrouillées et ne peuvent être supprimées.</li>
      <li><strong>Custom</strong> : dimensions ajoutées par l'utilisateur via l'interface. Elles sont créées dans <code>dim_custom_dimension</code> et ajoutées aux colonnes de <code>fact_entry</code> et <code>stg_entry</code> via <code>ALTER TABLE ADD COLUMN</code>.</li>
    </ul>

    <h3>Propagated vs Non-propagated</h3>
    <p>Toutes les dimensions du registre central sont <strong>propagées</strong> (elles passent à travers le pipeline). Seules <code>level</code> (niveau de stockage) et <code>amount</code> (mesure) ne sont pas propagées car elles ne sont pas des dimensions.</p>

    <h2>2. Catégories de dimensions</h2>
    <p>Le registre central (<code>dimensions.rs</code>) définit trois catégories, chacune portant des règles de comportement spécifiques :</p>

    <table className="help-table">
      <thead>
        <tr>
          <th>Catégorie</th>
          <th>Propagée</th>
          <th>Pilotable</th>
          <th>Nullable</th>
          <th>Grain de clôture</th>
          <th>Dans les totaux</th>
        </tr>
      </thead>
      <tbody>
        <tr><td><strong>Fixed</strong></td><td>oui</td><td>non</td><td>non</td><td>oui</td><td>oui</td></tr>
        <tr><td><strong>Active</strong></td><td>oui</td><td>oui</td><td>non</td><td>oui</td><td>oui</td></tr>
        <tr><td><strong>Analytical</strong></td><td>oui</td><td>oui</td><td>oui</td><td>oui</td><td><strong>non (voir §4)</strong></td></tr>
      </tbody>
    </table>

    <h3>Définitions</h3>
    <ul>
      <li><strong>Fixed</strong> : propagées, non pilotables (figées), non nullables, dans le grain de clôture. Ex: <code>phase</code>, <code>entry_period</code>, <code>period</code>, <code>currency</code>.</li>
      <li><strong>Active</strong> : propagées, pilotables (modifiables via règles), non nullables, dans le grain de clôture. Ex: <code>entity</code>, <code>account</code>, <code>flow</code>, <code>nature</code>.</li>
      <li><strong>Analytical</strong> : propagées, pilotables, nullables, dans le grain de clôture, <strong>hors totaux</strong> (sémantique "dont"). Ex: <code>partner</code>, <code>share</code>, <code>analysis</code>, <code>analysis2</code>, <strong>et toutes les dimensions custom</strong>.</li>
    </ul>

    <h3>Dimensions custom</h3>
    <p>Les dimensions créées par l'utilisateur sont <strong>toujours de catégorie Analytical</strong>. Elles sont donc nullables et obéissent à la sémantique "dont".</p>

    <h2>3. Catalogue des dimensions built-in</h2>
    <p>Les 12 dimensions built-in, dans l'ordre canonique des colonnes de <code>fact_entry</code> / <code>stg_entry</code> :</p>

    <table className="help-table">
      <thead>
        <tr>
          <th>Nom technique</th>
          <th>Catégorie</th>
          <th>Libellé</th>
          <th>Source (Valeurs depuis)</th>
        </tr>
      </thead>
      <tbody>
        <tr><td><code>phase</code></td><td>Fixed</td><td>Phase</td><td>scenario_categories</td></tr>
        <tr><td><code>entity</code></td><td>Active</td><td>Entité</td><td>entities</td></tr>
        <tr><td><code>entry_period</code></td><td>Fixed</td><td>Exercice</td><td>periods</td></tr>
        <tr><td><code>period</code></td><td>Fixed</td><td>Période</td><td>periods</td></tr>
        <tr><td><code>account</code></td><td>Active</td><td>Compte</td><td>accounts</td></tr>
        <tr><td><code>flow</code></td><td>Active</td><td>Flux</td><td>flows</td></tr>
        <tr><td><code>currency</code></td><td>Fixed</td><td>Devise</td><td>currencies</td></tr>
        <tr><td><code>nature</code></td><td>Active</td><td>Nature</td><td>natures</td></tr>
        <tr><td><code>partner</code></td><td>Analytical</td><td>Partenaire</td><td>entities (emprunté)</td></tr>
        <tr><td><code>share</code></td><td>Analytical</td><td>Titre</td><td>entities (emprunté)</td></tr>
        <tr><td><code>analysis</code></td><td>Analytical</td><td>Analyse 1</td><td>libre (texte)</td></tr>
        <tr><td><code>analysis2</code></td><td>Analytical</td><td>Analyse 2</td><td>libre (texte)</td></tr>
      </tbody>
    </table>

    <h3>Dimensions empruntées</h3>
    <p><strong>Partner</strong> et <strong>Share</strong> empruntent les valeurs de la master data <code>entities</code> : ils ne sont pas des axes autonomes mais des <strong>rôles</strong> sur la liste centrale des entités. L'appartenance au groupe se déduit de la table <code>sat_perimeter</code>.</p>

    <h3>Dimensions libres</h3>
    <p><strong>Analysis</strong> et <strong>Analysis2</strong> sont des <strong>dimensions libres</strong> : il n'y a pas de table master data associée. Les valeurs sont saisies en texte libre dans l'interface.</p>

    <h2>4. Dimensions analytiques et sémantique "dont"</h2>
    <p>Une ligne dont une dimension <strong>Analytical</strong> est renseignée est un <strong>« dont »</strong> (of which) de la ligne de même grain où cette dimension est <strong>NULL</strong>. Elle ne s'additionne <strong>jamais</strong> au total.</p>

    <h3>Conséquences pratiques</h3>
    <ul>
      <li><strong>Totaux</strong> (bilan, compte de résultat) : ne somment que les lignes <strong>principales</strong> — toutes les dimensions analytiques <code>IS NULL</code>.</li>
      <li><strong>Clôtures</strong> : les dimensions analytiques <strong>font partie du grain de clôture</strong>. Chaque « dont » obtient <strong>sa propre F99</strong>, et la clôture principale ne somme que les constituants principaux → <strong>pas de double compte</strong>.</li>
      <li><strong>Conversion / variations</strong> : les lignes « dont » subissent <strong>les mêmes automatismes</strong> que les lignes principales à leur propre grain.</li>
    </ul>

    <h3>Règle importante</h3>
    <p><strong>Une ligne principale ne doit jamais porter de valeur analytique</strong> (sinon elle disparaît des totaux). Exemple à proscrire : un identifiant d'audit posé sur <code>analysis2</code> de chaque écriture.</p>

    <h2>5. Dimensions custom</h2>
    <p>L'utilisateur peut créer des dimensions custom pour étendre le modèle de données. Ces dimensions sont toujours de catégorie <strong>Analytical</strong> (donc nullables et sujettes à la sémantique "dont").</p>

    <h3>Création</h3>
    <ul>
      <li>Via la page <strong>Dimensions</strong> → formulaire "Ajouter une dimension"</li>
      <li>API : <code>POST /api/dimensions</code> avec <code>{`{name, label}`}</code></li>
      <li>Opérations Rust : <code>ALTER TABLE fact_entry ADD COLUMN {`{name}`} TEXT</code> + <code>INSERT INTO dim_custom_dimension</code></li>
    </ul>

    <h3>Validation du nom</h3>
    <p>Le nom technique doit respecter les contraintes suivantes (implémenté dans <code>dimensions.rs::is_valid_custom_name</code>) :</p>
    <ul>
      <li>1 à 50 caractères</li>
      <li>Premier caractère : lettre ou underscore</li>
      <li>Reste : alphanumérique + underscore</li>
      <li>Pas un nom réservé (<code>level</code>, <code>amount</code>, <code>id</code>)</li>
      <li>Pas déjà utilisé (built-in ou custom existante)</li>
    </ul>

    <h3>Règles de nommage</h3>
    <p>Préfixes recommandés pour identifier la nature de la dimension :</p>
    <table className="help-table">
      <thead>
        <tr>
          <th>Préfixe</th>
          <th>Exemple</th>
          <th>Signification</th>
        </tr>
      </thead>
      <tbody>
        <tr><td><code>seg_</code></td><td><code>seg_produit</code></td><td>Segment produit</td></tr>
        <tr><td><code>geo_</code></td><td><code>geo_region</code></td><td>Géographie</td></tr>
        <tr><td><code>proj_</code></td><td><code>proj_code</code></td><td>Projet</td></tr>
        <tr><td><code>ana_</code></td><td><code>ana_centre_cout</code></td><td>Centre de coût</td></tr>
      </tbody>
    </table>

    <h3>Suppression</h3>
    <ul>
      <li>API : <code>DELETE /api/dimensions/{`{name}`}</code></li>
      <li>Opérations Rust : <code>ALTER TABLE fact_entry DROP COLUMN {`{name}`} TEXT</code> + <code>DELETE FROM dim_custom_dimension</code></li>
      <li>⚠️ <strong>Destructif</strong> : toutes les données de cette colonne sont perdues</li>
    </ul>

    <h3>Utilisation</h3>
    <p>Les dimensions custom apparaissent :</p>
    <ul>
      <li>Dans la page <strong>Dimensions</strong> (liste + actions)</li>
      <li>Comme axes de sélection dans l'<strong>éditeur de règles</strong> (modes direct, via caractéristique, via référence directe)</li>
      <li>Comme colonnes dans les <strong>rapports</strong> (grain de restitution)</li>
      <li>Dans les <strong>indicateurs</strong> (grain d'évaluation)</li>
    </ul>

    <h2>6. Master data — Structure générale</h2>
    <p>Le <strong>Master Data</strong> regroupe toutes les tables de référence qui alimentent les dimensions. Chaque table master data définit :</p>

    <h3>Clé primaire</h3>
    <ul>
      <li>Pour la plupart : colonne <code>code</code> (TEXT, ex: <code>entities.code</code>, <code>accounts.code</code>)</li>
      <li>Exception : <code>consolidations.id</code> (INTEGER auto-généré) car l'identité métier est la clé naturelle à 5 éléments</li>
    </ul>

    <h3>Colonnes</h3>
    <ul>
      <li><code>code</code> : identifiant technique (PK)</li>
      <li><code>libellé</code> : description lisible</li>
      <li><code>Attributs</code> : colonnes métier spécifiques (ex: <code>classe</code> pour les comptes, <code>devise_fonctionnelle</code> pour les entités)</li>
      <li><code>FK</code> : colonnes pointant vers d'autres master data (ex: <code>entity.devise_fonctionnelle → currencies</code>)</li>
    </ul>

    <h3>Colonnes dynamiques</h3>
    <p>Certains attributs sont ajoutés à l'exécution via deux mécanismes :</p>
    <ul>
      <li><strong>Caractéristiques N1/N2</strong> (patron A) : regroupements des membres d'une dimension</li>
      <li><strong>Références directes</strong> (patron B) : colonnes pointant directement vers une autre dimension</li>
    </ul>

    <h2>7. Tables master data natives</h2>
    <p>Les tables master data natives (built-in) sont exposées via <code>GET /api/md</code> et éditables dans la page <strong>Master Data</strong> :</p>

    <h3>Dimensions de la table de faits</h3>
    <table className="help-table">
      <thead>
        <tr>
          <th>Table API</th>
          <th>Table SQL</th>
          <th>Colonnes principales</th>
        </tr>
      </thead>
      <tbody>
        <tr><td><code>scenario_categories</code></td><td>dim_scenario_category</td><td>code, libellé</td></tr>
        <tr><td><code>entities</code></td><td>dim_entity</td><td>code, libellé, devise_fonctionnelle, entite_parent, statut</td></tr>
        <tr><td><code>periods</code></td><td>dim_period</td><td>code, libellé</td></tr>
        <tr><td><code>accounts</code></td><td>dim_account</td><td>code, libellé, classe, sous_classe, flow_scheme</td></tr>
        <tr><td><code>flows</code></td><td>dim_flow</td><td>code, libellé (dimension nue)</td></tr>
        <tr><td><code>currencies</code></td><td>dim_currency</td><td>code_iso, libellé, decimales</td></tr>
        <tr><td><code>natures</code></td><td>dim_nature</td><td>code, libellé, rules</td></tr>
      </tbody>
    </table>

    <h3>Référentiels de paramétrage</h3>
    <table className="help-table">
      <thead>
        <tr>
          <th>Table API</th>
          <th>Table SQL</th>
          <th>Colonnes principales</th>
        </tr>
      </thead>
      <tbody>
        <tr><td><code>methods</code></td><td>dim_method</td><td>code, libellé, consolidated</td></tr>
        <tr><td><code>sous_classes</code></td><td>dim_sous_classe</td><td>code, libelle, classe, sens</td></tr>
        <tr><td><code>variants</code></td><td>dim_variant</td><td>code, libellé</td></tr>
        <tr><td><code>rate_sets</code></td><td>dim_rate_set</td><td>code, libellé</td></tr>
        <tr><td><code>perimeter_sets</code></td><td>dim_perimeter_set</td><td>code, libellé</td></tr>
        <tr><td><code>flow_schemes</code></td><td>dim_flow_scheme</td><td>code, libellé</td></tr>
        <tr><td><code>flow_scheme_items</code></td><td>sat_flow_scheme_item</td><td>scheme, flow, taux_conversion, flux_ecart, flux_de_report, flux_a_nouveau</td></tr>
        <tr><td><code>consolidations</code></td><td>dim_consolidation</td><td>id (PK auto), libellé, phase, exercice, perimeter_set, variant, presentation_currency, …</td></tr>
      </tbody>
    </table>

    <h3>Tables satellites (non dimensions)</h3>
    <p>Elles ne sont pas des dimensions mais des tables de paramétrage :</p>
    <table className="help-table">
      <thead>
        <tr>
          <th>Table API</th>
          <th>Table SQL</th>
          <th>Colonnes principales</th>
        </tr>
      </thead>
      <tbody>
        <tr><td><code>perimeter</code></td><td>sat_perimeter</td><td>perimeter_set, entity, period, methode, pct_interet, pct_integration, entree, sortie</td></tr>
        <tr><td><code>rates</code></td><td>sat_exchange_rate</td><td>rate_set, currency_source, period, taux_close, taux_moyen, taux_ouverture</td></tr>
      </tbody>
    </table>

    <h3>Tables dynamiques (runtime)</h3>
    <p>Ces tables sont créées à l'exécution par l'utilisateur :</p>
    <table className="help-table">
      <thead>
        <tr>
          <th>Préfixe</th>
          <th>Description</th>
          <th>Exemple</th>
        </tr>
      </thead>
      <tbody>
        <tr><td><code>car_*</code></td><td>Table de valeurs d'une caractéristique N1</td><td><code>car_comportement</code></td></tr>
        <tr><td><code>lst_*</code></td><td>Liste de valeurs autonome</td><td><code>lst_incoterm</code></td></tr>
      </tbody>
    </table>

    <h2>8. Tables satellites</h2>
    <p>Les tables satellites portent les règles de consolidation et les paramètres multi-consolidations :</p>

    <h3>sat_perimeter (Périmètre de consolidation)</h3>
    <p>Définit la méthode d'intégration et les variations de périmètre par entité et période :</p>
    <ul>
      <li><strong>PK</strong> : <code>(perimeter_set, entity, period)</code></li>
      <li><strong>Champs</strong> : <code>methode</code> (FK dim_method), <code>pct_interet</code>, <code>pct_integration</code>, <code>entree</code>, <code>sortie</code></li>
      <li><strong>Utilisation</strong> : jointures dans les règles (scope périmètre), application du taux d'intégration natif</li>
    </ul>

    <h3>sat_flow_scheme_item (Articulation des flux)</h3>
    <p>Définit le comportement par flux pour chaque schéma de flux :</p>
    <ul>
      <li><strong>PK</strong> : <code>(scheme, flow)</code></li>
      <li><strong>Champs</strong> : <code>taux_conversion</code> (close_n1/avg/close_n), <code>flux_ecart</code>, <code>flux_de_report</code>, <code>flux_a_nouveau</code></li>
      <li><strong>Utilisation</strong> : conversion multi-devises, reconstruction des clôtures, report d'à-nouveau</li>
    </ul>

    <h3>sat_exchange_rate (Taux de change)</h3>
    <p>Taux de conversion vers la devise pivot :</p>
    <ul>
      <li><strong>PK</strong> : <code>(rate_set, currency_source, period)</code></li>
      <li><strong>Champs</strong> : <code>taux_close</code>, <code>taux_moyen</code>, <code>taux_ouverture</code> (= clôture N-1 portée par N)</li>
      <li><strong>Utilisation</strong> : conversion multi-devises, cross-rates pour la devise de présentation</li>
    </ul>

    <h2>9. API REST</h2>
    <h3>API Dimensions</h3>
    <ul>
      <li><code>GET /api/dimensions</code> — liste toutes les dimensions (built-in + custom) avec leurs propriétés</li>
      <li><code>POST /api/dimensions</code> — créer une dimension custom</li>
      <li><code>DELETE /api/dimensions/{`{name}`}</code> — supprimer une dimension custom</li>
    </ul>

    <h3>API Master Data</h3>
    <ul>
      <li><code>GET /api/md</code> — liste les tables navigables (natives + car_* + lst_*)</li>
      <li><code>GET /api/md/{`{table}`}/schema</code> — schéma complet d'une table (colonnes + FK)</li>
      <li><code>GET /api/md/{`{table}`}</code> — lister les lignes d'une table</li>
      <li><code>POST /api/md/{`{table}`}</code> — créer une ligne</li>
      <li><code>PUT /api/md/{`{table}`}</code> — mettre à jour une ligne</li>
      <li><code>DELETE /api/md/{`{table}`}</code> — supprimer une ligne</li>
      <li><code>POST /api/md/{`{table}`}/rename</code> — renommer le code d'un membre (si plus aucune référence ne le cite)</li>
    </ul>

    <h3>Validation des références</h3>
    <p>À l'écriture (<code>POST</code>/<code>PUT</code>), le serveur valide :</p>
    <ul>
      <li>Les FK pointent vers des codes existants dans les tables cibles</li>
      <li>Les champs obligatoires (non-nullable) sont renseignés</li>
      <li>L'auto-référence est tolérée (ex: <code>flux_de_report = F99</code> sur la ligne F99)</li>
    </ul>

    <h2>10. Interface utilisateur</h2>
    <h3>Page Dimensions</h3>
    <p>Liste toutes les dimensions avec leurs propriétés :</p>
    <ul>
      <li>Nom technique, libellé, catégorie</li>
      <li><strong>Valeurs depuis</strong> : table master data source (ou "libre" pour les axes texte)</li>
      <li><strong>Perso.</strong> : custom ou built-in</li>
      <li><strong>Pilotable</strong> : oui/non</li>
      <li><strong>Actions</strong> : supprimer (pour les custom uniquement)</li>
    </ul>
    <p>Formulaire de création : nom technique (validation regex) + libellé.</p>

    <h3>Page Master Data</h3>
    <p>CRUD générique sur les tables master data :</p>
    <ul>
      <li>Sélecteur de table (groupé : Dimensions, Référentiels de paramétrage)</li>
      <li>Grille avec tri sur les colonnes</li>
      <li>Ajout / Édition / Suppression (avec validation des FK)</li>
      <li><strong>Renommer</strong> : bouton pour changer le code d'un membre (si plus aucune référence ne le cite)</li>
      <li><strong>Santé des données</strong> : bouton pour vérifier l'intégrité référentielle (orphelins)</li>
    </ul>
    <p>Vues de dimensions empruntées : <code>partner</code> et <code>share</code> sont exposés comme vues en lecture seule sur <code>entities</code>.</p>

    <h2>11. Exemples concrets</h2>
    <h3>Exemple 1 : Créer une dimension custom</h3>
    <p>Objectif : ajouter un axe "Segment produit" pour analyser les ventes par ligne de produits.</p>
    <ol>
      <li>Aller dans la page <strong>Dimensions</strong></li>
      <li>Cliquer sur <strong>Ajouter une dimension</strong></li>
      <li>Saisir <code>Nom technique</code> : <code>seg_produit</code></li>
      <li>Saisir <code>Libellé</code> : <code>Segment produit</code></li>
      <li>Cliquer sur <strong>Créer la dimension</strong></li>
    </ol>
    <p>Résultat :</p>
    <ul>
      <li>Une colonne <code>seg_produit</code> est ajoutée à <code>fact_entry</code> et <code>stg_entry</code></li>
      <li>La dimension apparaît dans la liste Dimensions (catégorie Analytical, perso: oui)</li>
      <li>Elle peut être utilisée dans les règles comme axe de sélection ou de destination</li>
    </ul>

    <h3>Exemple 2 : Gérer le master data des entités</h3>
    <p>Objectif : créer une nouvelle entité filiale avec sa devise fonctionnelle.</p>
    <ol>
      <li>Aller dans la page <strong>Master Data</strong></li>
      <li>Sélectionner la table <strong>Entités</strong></li>
      <li>Cliquer sur <strong>Ajouter</strong></li>
      <li>Saisir :
        <ul>
          <li><code>Code</code> : <code>FILIALE_X</code></li>
          <li><code>Libellé</code> : <code>Filiale X S.A.</code></li>
          <li><code>Devise fonct.</code> : <code>USD</code> (dropdown depuis <code>Devises</code>)</li>
          <li><code>Statut</code> : <code>actif</code></li>
        </ul>
      </li>
      <li>Cliquer sur <strong>Enregistrer</strong></li>
    </ol>
    <p>Le serveur valide que la devise <code>USD</code> existe dans <code>dim_currency</code> avant d'insérer.</p>

    <h3>Exemple 3 : Renommer un compte</h3>
    <p>Objectif : corriger une faute de frappe dans un code de compte.</p>
    <ol>
      <li>Aller dans <strong>Master Data</strong> → <strong>Comptes</strong></li>
      <li>Trouver la ligne avec <code>Code</code> = <code>601AV</code> (au lieu de <code>601AV</code>)</li>
      <li>Cliquer sur le bouton <strong>Renommer</strong></li>
      <li>Saisir le nouveau code : <code>601AV</code></li>
      <li>Cliquer sur <strong>OK</strong></li>
    </ol>
    <p>Si le compte est référencé dans des écritures ou des règles, le renommage est <strong>bloqué</strong> avec un message indiquant où il est cité.</p>

    <h3>Voir aussi</h3>
    <ul>
      <li><NavLink to='regles'>Utilisation des dimensions dans les règles</NavLink></li>
      <li><NavLink to='schemas-flux'>Flux et schémas de flux</NavLink></li>
      <li><NavLink to='taux-integration'>Périmètre et taux d'intégration</NavLink></li>
    </ul>
  </div>
);

const importSaisieContent = (
  <div className="help-content">
    <h2>Vue d'ensemble</h2>
    <p>
      Le système offre <strong>deux modes d'entrée</strong> pour alimenter la consolidation en données : l'<strong>import CSV</strong> et la <strong>saisie manuelle</strong>. Les deux modes écrivent dans la table <code>stg_entry</code> (niveau <code>raw</code>), qui sert de source au pipeline de consolidation. Le grain de saisie est la <strong>remontée</strong> : <code>Phase + Entry_period</code>.
    </p>

    <h2>1. Deux modes d'entrée</h2>
    <table className="help-table">
      <thead>
        <tr>
          <th>Mode</th>
          <th>Usage</th>
          <th>Volume</th>
          <th>Fréquence</th>
        </tr>
      </thead>
      <tbody>
        <tr><td><strong>Import CSV</strong></td><td>Import massif de liasses comptables</td><td>Milliers à millions de lignes</td><td>Quotidien/mensuel</td></tr>
        <tr><td><strong>Saisie manuelle</strong></td><td>Écritures ponctuelles / ajustements</td><td>Dizaines à centaines de lignes</td><td>À la demande</td></tr>
      </tbody>
    </table>

    <h3>Staging → Pipeline</h3>
    <p>Les deux modes écrivent dans <code>stg_entry</code> (staging). Le pipeline lit cette table et produit les niveaux <code>corporate</code>, <code>converted</code> et <code>consolidated</code> dans <code>fact_entry</code>.</p>
    <pre>
      {`stg_entry (raw)
         ↓ A. Agrégation
      fact_entry (corporate)
         ↓ B. Conversion
      fact_entry (converted)
         ↓ C. Consolidation
      fact_entry (consolidated)`}
    </pre>

    <h2>2. Format CSV</h2>
    <h3>Ordre et définition des colonnes</h3>
    <p>Le CSV doit respecter l'ordre exact des colonnes :</p>
    <pre>
      {`Phase, Entity, Entry_period, Period, Account, Flow, Currency, Nature, Partner*, Share*, Analysis*, Analysis2*, Source*, Amount`}
    </pre>

    <h3>Champs obligatoires</h3>
    <p>Les 9 premiers champs + Amount sont obligatoires :</p>
    <table className="help-table">
      <thead>
        <tr>
          <th>Champ</th>
          <th>Type</th>
          <th>Description</th>
        </tr>
      </thead>
      <tbody>
        <tr><td><code>Phase</code></td><td>TEXT</td><td>Phase de consolidation (code depuis <code>scenario_categories</code>)</td></tr>
        <tr><td><code>Entity</code></td><td>TEXT</td><td>Entité émettrice (code depuis <code>entities</code>)</td></tr>
        <tr><td><code>Entry_period</code></td><td>TEXT</td><td>Exercice en cours (période de saisie, code depuis <code>periods</code>)</td></tr>
        <tr><td><code>Period</code></td><td>TEXT</td><td>Période impactée (période de rattachement, code depuis <code>periods</code>)</td></tr>
        <tr><td><code>Account</code></td><td>TEXT</td><td>Compte (code depuis <code>accounts</code>)</td></tr>
        <tr><td><code>Flow</code></td><td>TEXT</td><td>Flux (code depuis <code>flows</code>, ex. F00, F20, F99)</td></tr>
        <tr><td><code>Currency</code></td><td>TEXT</td><td>Devise (code ISO depuis <code>currencies</code>)</td></tr>
        <tr><td><code>Nature</code></td><td>TEXT</td><td>Nature de l'écriture (code depuis <code>natures</code>, obligatoire)</td></tr>
        <tr><td><code>Amount</code></td><td>DECIMAL(18,2)</td><td>Montant de l'écriture</td></tr>
      </tbody>
    </table>

    <h3>Champs optionnels</h3>
    <p>Les 5 champs marqués d'un <code>*</code> sont optionnels. Absents du header, ils sont insérés à <code>NULL</code> :</p>
    <table className="help-table">
      <thead>
        <tr>
          <th>Champ</th>
          <th>Type</th>
          <th>Description</th>
        </tr>
      </thead>
      <tbody>
        <tr><td><code>Partner*</code></td><td>TEXT</td><td>Contrepartie (interco ou tiers, code depuis <code>entities</code>)</td></tr>
        <tr><td><code>Share*</code></td><td>TEXT</td><td>Participation (code depuis <code>entities</code>)</td></tr>
        <tr><td><code>Analysis*</code></td><td>TEXT</td><td>Axe analytique libre 1 (texte libre)</td></tr>
        <tr><td><code>Analysis2*</code></td><td>TEXT</td><td>Axe analytique libre 2 (texte libre)</td></tr>
        <tr><td><code>Source*</code></td><td>TEXT</td><td>Métadonnée de provenance (réf. liasse source, etc.) — NON propagée par le pipeline</td></tr>
      </tbody>
    </table>

    <h3>Exemple de fichier CSV</h3>
    <pre>
      {`Phase,Entity,Entry_period,Period,Account,Flow,Currency,Nature,Partner,Share,Analysis,Analysis2,Source,Amount
REEL,A,2024,2024,100,F00,USD,0LIASS,,,,LIASSE_A_2024,1000.00
REEL,A,2024,2024,700,F20,USD,0LIASS,,,,LIASSE_A_2024,500.00
REEL,B,2024,2024,100,F00,EUR,0LIASS,,,,LIASSE_B_2024,2000.00
REEL,A,2024,2024,400,F20,USD,1AJUST,B,,,AJUST_CORRECTION,-100.00`}
    </pre>

    <h2>3. Staging vs Fact_entry</h2>
    <h3>Table stg_entry</h3>
    <p>Table de staging qui reçoit la saisie brute (CSV ou manuelle) :</p>
    <table className="help-table">
      <thead>
        <tr>
          <th>Caractéristique</th>
          <th>Valeur</th>
        </tr>
      </thead>
      <tbody>
        <tr><td><strong>Role</strong></td><td>Saisie brute, niveau <code>raw</code></td></tr>
        <tr><td><strong>Grain</strong></td><td><code>Phase + Entry_period</code> (remontée)</td></tr>
        <tr><td><strong>PK</strong></td><td><code>id</code> (auto via <code>seq_stg_entry</code>)</td></tr>
        <tr><td><strong>Columns</strong></td><td>Dimensions propagées + <code>amount</code> + <code>source</code> (métadonnée)</td></tr>
        <tr><td><strong>Dimensions</strong></td><td>Sans <code>level</code> ni <code>consolidation_id</code> (ajoutés par le pipeline)</td></tr>
      </tbody>
    </table>

    <h3>Table fact_entry</h3>
    <p>Table de faits produite par le pipeline :</p>
    <table className="help-table">
      <thead>
        <tr>
          <th>Caractéristique</th>
          <th>Valeur</th>
        </tr>
      </thead>
      <tbody>
        <tr><td><strong>Role</strong></td><td>Écritures consolidées aux 3 niveaux</td></tr>
        <tr><td><strong>Niveaux</strong></td><td><code>corporate</code>, <code>converted</code>, <code>consolidated</code></td></tr>
        <tr><td><strong>PK</strong></td><td><code>id</code> (auto via <code>seq_entry</code>)</td></tr>
        <tr><td><strong>Columns</strong></td><td>Dimensions en <code>id</code> technique + <code>level</code> + <code>consolidation_id</code> + <code>amount</code></td></tr>
        <tr><td><strong>Isolation</strong></td><td>Chaque run isolé par <code>consolidation_id</code></td></tr>
      </tbody>
    </table>

    <h2>4. Import CSV</h2>
    <h3>Mécanique d'import</h3>
    <ol>
      <li>Upload du fichier CSV via l'interface</li>
      <li>Le serveur valide le header (colonnes requises présentes)</li>
      <li>Validation des références FK (anti-jointure avant insertion)</li>
      <li>Écriture dans un fichier temporaire</li>
      <li>Chargement via <code>read_csv_auto</code> avec <code>INSERT INTO stg_entry</code></li>
      <li>Suppression du fichier temporaire</li>
    </ol>

    <h3>Validation des références</h3>
    <p>Pour chaque colonne qui est une référence vers une master data, le serveur vérifie que <strong>toutes les valeurs</strong> présentes dans le CSV existent dans la table cible :</p>
    <ul>
      <li><code>phase</code> → <code>dim_scenario_category.code</code></li>
      <li><code>entity</code> → <code>dim_entity.code</code></li>
      <li><code>entry_period</code> → <code>dim_period.code</code></li>
      <li><code>period</code> → <code>dim_period.code</code></li>
      <li><code>account</code> → <code>dim_account.code</code></li>
      <li><code>flow</code> → <code>dim_flow.code</code></li>
      <li><code>currency</code> → <code>dim_currency.code_iso</code></li>
      <li><code>nature</code> → <code>dim_nature.code</code></li>
      <li><code>partner</code> → <code>dim_entity.code</code></li>
      <li><code>share</code> → <code>dim_entity.code</code></li>
    </ul>
    <p>Si une valeur est absente, l'import échoue avec un message explicite : <code>"partner : valeur(s) absente(s) de entities → X, Y, Z"</code>.</p>

    <h3>Gestion des erreurs</h3>
    <p>Types d'erreurs possibles :</p>
    <table className="help-table">
      <thead>
        <tr>
          <th>Erreur</th>
          <th>Cause</th>
          <th>Message</th>
        </tr>
      </thead>
      <tbody>
        <tr><td><code>colonne absente</code></td><td>Header incomplet</td><td>"colonne absente du header : entity"</td></tr>
        <tr><td><code>fichier vide</code></td><td>Upload vide</td><td>"fichier vide"</td></tr>
        <tr><td><code>référence invalide</code></td><td>Valeur absente de la master data</td><td>"partner : valeur(s) absente(s) de entities → X, Y"</td></tr>
        <tr><td><code>fichier non UTF-8</code></td><td>Encodage incorrect</td><td>"fichier non UTF-8 : …"</td></tr>
      </tbody>
    </table>

    <h2>5. Saisie manuelle</h2>
    <h3>Interface</h3>
    <p>La page <strong>Saisie</strong> offre deux sections :</p>
    <ol>
      <li><strong>Nouvelles saisies</strong> : grille inline type Excel pour saisir un batch d'écritures</li>
      <li><strong>Saisies manuelles enregistrées</strong> : liste des écritures avec <code>source = MANUAL</code> (éditable/supprimable)</li>
    </ol>

    <h3>En-tête commun</h3>
    <p>Un en-tête avec 6 champs factorisés pré-remplit chaque nouvelle ligne :</p>
    <table className="help-table">
      <thead>
        <tr>
          <th>Champ</th>
          <th>Description</th>
        </tr>
      </thead>
      <tbody>
        <tr><td><code>Phase</code></td><td>Phase de consolidation</td></tr>
        <tr><td><code>Entity</code></td><td>Entité émettrice</td></tr>
        <tr><td><code>Entry_period</code></td><td>Exercice en cours</td></tr>
        <tr><td><code>Period</code></td><td>Période impactée</td></tr>
        <tr><td><code>Currency</code></td><td>Devise</td></tr>
        <tr><td><code>Nature</code></td><td>Nature de l'écriture</td></tr>
      </tbody>
    </table>
    <p>Les champs variables par ligne sont : <code>account</code>, <code>flow</code>, <code>partner</code>, <code>share</code>, <code>analysis</code>, <code>analysis2</code>, <code>amount</code>.</p>

    <h3>Validation locale</h3>
    <p>Avant envoi, l'interface valide :</p>
    <ul>
      <li>Champs obligatoires renseignés</li>
      <li>Montant numérique (accepte point ou virgule décimale)</li>
    </ul>
    <p>Le serveur valide en plus les FK (références master data).</p>

    <h3>Enregistrement</h3>
    <p>L'envoi se fait via <code>POST /api/entries</code> avec un tableau d'objets <code>EntryInput</code> :</p>
    <pre>
      {`POST /api/entries
      [
        {
          "phase": "REEL",
          "entity": "A",
          "entry_period": "2024",
          "period": "2024",
          "account": "400",
          "flow": "F20",
          "currency": "USD",
          "nature": "1AJUST",
          "partner": "",
          "share": "",
          "analysis": "",
          "analysis2": "",
          "amount": "100.50"
        }
      ]`}
    </pre>

    <h3>Édition et suppression</h3>
    <p>Les écritures avec <code>source = MANUAL</code> sont éditables et supprimables :</p>
    <ul>
      <li><code>PUT /api/entries/{`{id}`}</code> : modifier une écriture</li>
      <li><code>DELETE /api/entries/{`{id}`}</code> : supprimer une écriture</li>
    </ul>
    <p>⚠️ <strong>Protection</strong> : Les écritures importées via CSV (<code>source ≠ MANUAL</code>) ne peuvent être ni éditées ni supprimées.</p>

    <h2>6. Grain de saisie</h2>
    <h3>Grain remontée (Phase + Entry_period)</h3>
    <p>Les saisies sont au grain <strong>remontée</strong> :</p>
    <ul>
      <li><code>Phase</code> : remplace l'ancien <code>Scenario</code></li>
      <li><code>Entry_period</code> : exercice comptable en cours (la clôture travaillée)</li>
    </ul>
    <p>Ce grain est <strong>partagé</strong> entre toutes les consolidations qui le consomment. <code>fact_entry</code> référence la consolidation par <code>consolidation_id</code> pour isoler chaque run.</p>

    <h3>Période vs Entry_period</h3>
    <table className="help-table">
      <thead>
        <tr>
          <th>Champ</th>
          <th>Role</th>
          <th>Exemple</th>
        </tr>
      </thead>
      <tbody>
        <tr><td><code>Entry_period</code></td><td>Exercice en cours (saisie)</td><td>2024</td></tr>
        <tr><td><code>Period</code></td><td>Période impactée (rattachement)</td><td>2024, 2025, 2026 (plan à 3 ans)</td></tr>
      </tbody>
    </table>
    <p>Pour une consolidation mono-annuelle, <code>Entry_period</code> et <code>Period</code> sont souvent identiques. Dans un plan multi-annuel, <code>Entry_period</code> est constant (ex. 2024) alors que <code>Period</code> varie (2024, 2025, 2026).</p>

    <h2>7. Source (métadonnée de provenance)</h2>
    <h3>Remplacement de Audit_id</h3>
    <p>Le champ <code>Source</code> remplace l'ancien <code>Audit_id</code> :</p>
    <ul>
      <li><strong>Ancien</strong> : <code>Audit_id</code> était une dimension (propagée)</li>
      <li><strong>Nouveau</strong> : <code>Source</code> est une <strong>métadonnée non-dimensionnelle</strong></li>
    </ul>
    <p>Conséquences :</p>
    <ul>
      <li><code>Source</code> est <strong>hors registre des dimensions</strong> (non propagée par le pipeline)</li>
      <li><code>Source</code> est <strong>hors grain de clôture</strong></li>
      <li><code>Source</code> n'entre pas dans les totaux (pas de double compte)</li>
    </ul>

    <h3>Sémantique</h3>
    <p><code>Source</code> porte une référence de provenance :</p>
    <ul>
      <li>Import CSV : référence de la liasse source (ex. <code>"LIASSE_A_2024"</code>)</li>
      <li>Règles : identifiant de la règle génératrice</li>
      <li>Saisie manuelle : <code>"MANUAL"</code> (valeur par défaut)</li>
    </ul>

    <h2>8. API REST</h2>
    <h3>Import CSV</h3>
    <ul>
      <li><code>POST /api/import/entries</code> — import écritures (multipart, champ <code>file</code>)</li>
      <li><code>POST /api/import/rates</code> — import taux de change</li>
      <li><code>POST /api/import/perimeter</code> — import périmètre</li>
    </ul>

    <h3>Saisie manuelle</h3>
    <ul>
      <li><code>POST /api/entries</code> — créer des écritures (batch)</li>
      <li><code>PUT /api/entries/{`{id}`}</code> — modifier une écriture (source = MANUAL uniquement)</li>
      <li><code>DELETE /api/entries/{`{id}`}</code> — supprimer une écriture (source = MANUAL uniquement)</li>
      <li><code>GET /api/entries?level=raw&source=MANUAL</code> — lister les saisies manuelles</li>
    </ul>

    <h2>9. UI Import</h2>
    <h3>Page Import</h3>
    <p>La page <strong>Import</strong> offre 3 zones d'upload :</p>
    <ol>
      <li><strong>Liasses (écritures)</strong> : ajout (append) dans <code>stg_entry</code></li>
      <li><strong>Taux de change</strong> : upsert dans <code>sat_exchange_rate</code></li>
      <li><strong>Périmètre</strong> : upsert dans <code>sat_perimeter</code></li>
    </ol>

    <h3>Upload d'un fichier</h3>
    <ol>
      <li>Sélectionner le fichier CSV via le sélecteur</li>
      <li>Cliquer sur <strong>Importer</strong></li>
      <li>Attendre la validation et le chargement</li>
      <li>Voir le rapport de succès (nombre de lignes importées) ou d'erreur</li>
    </ol>

    <h3>Format CSV affiché</h3>
    <p>Chaque zone affiche le format attendu du header pour guider l'utilisateur :</p>
    <pre>
      {`Liasses (écritures) :
      Phase, Entity, Entry_period, Period, Account, Flow, Currency, Nature,
      Partner*, Share*, Analysis*, Analysis2*, Source*, Amount`}
    </pre>

    <h2>10. UI Saisie</h2>
    <h3>Section Nouvelles saisies</h3>
    <ul>
      <li><strong>En-tête commun</strong> : 6 champs factorisés qui pré-remplissent chaque ligne</li>
      <li><strong>Bouton "Appliquer partout"</strong> : propage les valeurs de l'en-tête aux lignes existantes</li>
      <li><strong>Toggle "Afficher les colonnes communes"</strong> : affiche/cache les 6 champs communs dans la grille</li>
      <li><strong>Grille</strong> : colonnes variables par ligne + bouton de suppression</li>
      <li><strong>Validation locale</strong> : champs obligatoires + montant numérique</li>
      <li><strong>Enregistrement</strong> : <code>source = MANUAL</code>, message de confirmation avec IDs</li>
    </ul>

    <h3>Section Saisies manuelles enregistrées</h3>
    <ul>
      <li>Liste des écritures avec <code>source = MANUAL</code></li>
      <li>Bouton <strong>✎</strong> : éditer dans une modale</li>
      <li>Bouton <strong>✕</strong> : supprimer avec confirmation</li>
      <li>Filtre par <code>level = raw</code> (lecture de <code>stg_entry</code>)</li>
    </ul>

    <h3>Autocomplétion</h3>
    <p>Les champs avec une master data associée affichent une liste déroulante avec autocomplétion :</p>
    <ul>
      <li><code>phase</code> → <code>scenario_categories</code></li>
      <li><code>entity</code> → <code>entities</code></li>
      <li><code>entry_period</code> → <code>periods</code></li>
      <li><code>period</code> → <code>periods</code></li>
      <li><code>account</code> → <code>accounts</code></li>
      <li><code>flow</code> → <code>flows</code></li>
      <li><code>currency</code> → <code>currencies</code></li>
      <li><code>nature</code> → <code>natures</code></li>
      <li><code>partner</code> → <code>entities</code></li>
      <li><code>share</code> → <code>entities</code></li>
    </ul>
    <p>Les champs libres (<code>analysis</code>, <code>analysis2</code>) sont des saisies texte libres.</p>

    <h2>11. Exemples concrets</h2>
    <h3>Exemple 1 : Import d'une liasse comptable</h3>
    <p>Objectif : importer les écritures de la filiale A pour l'exercice 2024.</p>
    <ol>
      <li>Préparer le fichier CSV <code>liasse_A_2024.csv</code> avec le contenu :
        <pre>
          {`Phase,Entity,Entry_period,Period,Account,Flow,Currency,Nature,Partner,Share,Analysis,Analysis2,Source,Amount
REEL,A,2024,2024,100,F00,USD,0LIASS,,,,LIASSE_A_2024,10000.00
REEL,A,2024,2024,700,F20,USD,0LIASS,,,,LIASSE_A_2024,5000.00
REEL,A,2024,2024,400,F20,USD,0LIASS,,,,LIASSE_A_2024,-2000.00`}
        </pre>
      </li>
      <li>Aller dans la page <strong>Import</strong></li>
      <li>Sélectionner <code>liasse_A_2024.csv</code></li>
      <li>Cliquer sur <strong>Importer</strong></li>
      <li>Voir le rapport : <code>"3 ligne(s) importée(s)."</code></li>
      <li>Relancer le pipeline pour consolider</li>
    </ol>

    <h3>Exemple 2 : Saisie d'un ajustement manuel</h3>
    <p>Objectif : corriger une erreur sur le compte 400 de l'entité A (débit au lieu de crédit).</p>
    <ol>
      <li>Aller dans la page <strong>Saisie</strong></li>
      <li>Dans l'en-tête commun, saisir :
        <ul>
          <li><code>Phase</code> : <code>REEL</code></li>
          <li><code>Entity</code> : <code>A</code></li>
          <li><code>Entry_period</code> : <code>2024</code></li>
          <li><code>Period</code> : <code>2024</code></li>
          <li><code>Currency</code> : <code>USD</code></li>
          <li><code>Nature</code> : <code>1AJUST</code></li>
        </ul>
      </li>
      <li>Dans la grille, saisir :
        <ul>
          <li><code>Account</code> : <code>400</code></li>
          <li><code>Flow</code> : <code>F20</code></li>
          <li><code>Amount</code> : <code>100.00</code></li>
        </ul>
      </li>
      <li>Cliquer sur <strong>Enregistrer tout</strong></li>
      <li>Voir le rapport : <code>"1 écriture(s) enregistrée(s) (IDs : 123). Cible : stg_entry (niveau raw). Relancez le pipeline pour propager."</code></li>
      <li>Relancer le pipeline pour consolider</li>
    </ol>

    <h3>Exemple 3 : Édition d'une saisie manuelle</h3>
    <p>Objectif : corriger le montant de l'ajustement précédent (100.00 → 150.00).</p>
    <ol>
      <li>Aller dans la page <strong>Saisie</strong></li>
      <li>Dans la section <strong>Saisies manuelles enregistrées</strong>, trouver l'écriture ID 123</li>
      <li>Cliquer sur le bouton <strong>✎</strong></li>
      <li>Dans la modale, modifier <code>Amount</code> : <code>150.00</code></li>
      <li>Cliquer sur <strong>Enregistrer</strong></li>
      <li>Relancer le pipeline pour consolider</li>
    </ol>

    <h3>Voir aussi</h3>
    <ul>
      <li><NavLink to='dimensions-masterdata'>Dimensions et Master Data (référentiels de validation)</NavLink></li>
      <li><NavLink to='schemas-flux'>Flux de consolidation (Flow)</NavLink></li>
      <li><NavLink to='taux-integration'>Périmètre et taux d'intégration</NavLink></li>
    </ul>
  </div>
);

const tauxChangeContent = (
  <div className="help-content">
    <h2>Vue d'ensemble</h2>
    <p>
      Les <strong>taux de change</strong> permettent la <strong>conversion multi-devises</strong> lors de l'étape C du pipeline de consolidation. Chaque entité travaille dans sa devise fonctionnelle, et le moteur convertit toutes les écritures vers la devise de présentation du groupe, en générant automatiquement les écarts de conversion (F80, F81).
    </p>

    <h2>1. Définition</h2>
    <h3>Qu'est-ce qu'un jeu de taux ?</h3>
    <p>Un <strong>jeu de taux</strong> (<code>dim_rate_set</code>) est un catalogue de taux de change pour une période donnée. Il permet de distinguer différents scénarios : taux réels, taux budget, taux prévisionnels, etc. Une consolidation référence un jeu de taux via <code>consolidation.rate_set</code>.</p>

    <h3>Devise pivot</h3>
    <p>Tous les taux stockés dans <code>sat_exchange_rate</code> convertissent une devise vers la <strong>devise pivot</strong> de l'instance (lue dans <code>app_config.pivot_currency</code>). Pour passer d'une devise fonctionnelle vers une devise de présentation, le moteur calcule un <strong>cross-rate</strong> :</p>
    <pre>
      {`taux_cross(fonctionnelle → présentation)
          = taux(fonctionnelle → pivot) / taux(présentation → pivot)`}
    </pre>
    <p><em>Cas particuliers : si fonctionnelle = présentation → taux = 1.0 ; si présentation = pivot → cross = taux_fonctionnelle ; si fonctionnelle = pivot → cross = 1 / taux_présentation.</em></p>

    <h2>2. Structure de la table des taux</h2>
    <h3>sat_exchange_rate</h3>
    <p>Table satellite stockant les taux de change vers la devise pivot :</p>
    <table className="help-table">
      <thead>
        <tr>
          <th>Champ</th>
          <th>Type</th>
          <th>Rôle</th>
        </tr>
      </thead>
      <tbody>
        <tr><td><code>rate_set</code></td><td>INTEGER</td><td>Clé du jeu de taux (FK dim_rate_set.id, partie de PK)</td></tr>
        <tr><td><code>currency_source</code></td><td>TEXT</td><td>Devise source (code ISO, convertie vers le pivot, partie de PK)</td></tr>
        <tr><td><code>period</code></td><td>TEXT</td><td>Période du taux (partie de PK)</td></tr>
        <tr><td><code>taux_close</code></td><td>DECIMAL(18,8)</td><td>Taux de clôture N (obligatoire)</td></tr>
        <tr><td><code>taux_moyen</code></td><td>DECIMAL(18,8)</td><td>Taux moyen N (nullable, optionnel)</td></tr>
        <tr><td><code>taux_ouverture</code></td><td>DECIMAL(18,8)</td><td>Taux d'ouverture N (= clôture N-1, nullable, optionnel)</td></tr>
      </tbody>
    </table>
    <p><em>La PK est <code>(rate_set, currency_source, period)</code> : un même couple (source, période) peut exister dans plusieurs jeux de taux.</em></p>

    <h3>Tables associées</h3>
    <ul>
      <li><code>dim_rate_set</code> : catalogue des jeux de taux (code, libellé)</li>
      <li><code>app_config</code> : contient <code>pivot_currency</code> (ex. EUR)</li>
    </ul>

    <h2>3. Étape C du pipeline : Conversion multi-devises</h2>
    <h3>Principe</h3>
    <p>L'étape C (<code>pipeline::step_c</code>) convertit les écritures du niveau <code>corporate</code> (devise fonctionnelle) vers le niveau <code>converted</code> (devise de présentation). Pour chaque ligne :</p>
    <ol>
      <li>Résolution du taux du flux via le schéma de flux (<code>v_flow_behavior.taux_conversion</code>)</li>
      <li>Calcul du taux de conversion (cross-rate fonctionnelle → présentation)</li>
      <li>Montant converti = <code>amount × taux_flux</code></li>
      <li>Calcul de l'écart de conversion = <code>amount × (taux_report − taux_flux)</code>, posté sur le <code>flux_ecart</code></li>
      <li>Les lignes en devise de présentation sont copiées directement (taux = 1.0, écart = 0)</li>
    </ol>

    <h3>Taux de report</h3>
    <p><code>taux_report</code> est le taux du <strong>flux de report</strong> du flux (la clôture où il se solde), résolu par compte via <code>v_flow_behavior.flux_de_report</code>. Le cas usuel : le flux se reporte sur F99 en taux close_n, donc <code>taux_report = close_n</code>.</p>

    <h2>4. Mécanique de conversion par flux</h2>
    <h3>Types de taux disponibles</h3>
    <table className="help-table">
      <thead>
        <tr>
          <th>Valeur</th>
          <th>Source dans sat_exchange_rate</th>
          <th>Utilisation typique</th>
        </tr>
      </thead>
      <tbody>
        <tr><td><code>close_n1</code></td><td>taux_ouverture N (clôture N-1 portée par N)</td><td>F00 (ouverture), F01 (entrée)</td></tr>
        <tr><td><code>avg</code></td><td>taux_moyen N</td><td>F20 (variation)</td></tr>
        <tr><td><code>close_n</code></td><td>taux_close N</td><td>F80, F81, F98, F99</td></tr>
      </tbody>
    </table>

    <h3>Écarts de conversion</h3>
    <p>Les écarts sont générés automatiquement pour les flux porteurs d'un <code>flux_ecart</code> (non null). La formule générique :</p>
    <pre>
{`écart = amount × (taux_report − taux_flux)`}
    </pre>

    <h3>Cas particuliers (flux standards)</h3>
    <table className="help-table">
      <thead>
        <tr>
          <th>Flux</th>
          <th>Taux (taux_flux)</th>
          <th>Taux report</th>
          <th>Écart →</th>
          <th>Formule</th>
        </tr>
      </thead>
      <tbody>
        <tr><td><strong>F00</strong> (ouverture)</td><td>close_n1</td><td>close_n</td><td>F80</td><td><code>A × (close_n − close_n1)</code></td></tr>
        <tr><td><strong>F20</strong> (variation)</td><td>avg</td><td>close_n</td><td>F81</td><td><code>A × (close_n − avg)</code></td></tr>
        <tr><td><strong>F99</strong> (clôture)</td><td>close_n</td><td>close_n</td><td>0</td><td><code>A × (close_n − close_n) = 0</code></td></tr>
      </tbody>
    </table>

    <h2>5. Résolution du taux par compte (schémas de flux)</h2>
    <h3>Principe</h3>
    <p>Le comportement d'un flux dépend du compte via son <strong>schéma de flux</strong> (<code>dim_account.flow_scheme</code>). La résolution se fait en cascade :</p>
    <pre>
{`Compte → flow_scheme → sat_flow_scheme_item → v_flow_behavior
                              ↓ (taux_conversion, flux_ecart, flux_de_report)`}
    </pre>

    <h3>Schéma BILAN</h3>
    <p>Comptes de bilan : conversion avec écarts F80/F81.</p>
    <ul>
      <li>F00, F01 → taux close_n1, écart → F80</li>
      <li>F20 → taux avg, écart → F81</li>
      <li>F80, F81, F98, F99 → taux close_n, pas d'écart propre</li>
      <li>F99 s'auto-référence (flux_de_report = F99)</li>
    </ul>

    <h3>Schéma RESULTAT</h3>
    <p>Comptes de résultat : <strong>taux moyen sans écart</strong>.</p>
    <ul>
      <li>Tous les flux (F00, F20, F99) → taux avg</li>
      <li><code>flux_ecart = NULL</code> pour tous les flux (pas d'écart de conversion)</li>
      <li><code>flux_a_nouveau = NULL</code> (le résultat ne se reporte pas à l'exercice suivant)</li>
    </ul>

    <h3>Vue v_flow_behavior</h3>
    <p>Vue résolvant le comportement par compte, consommée par <code>pipeline::convert</code>, <code>materialize_closures</code> et <code>a_nouveau</code>. Expose les ids techniques (INTEGER) pour les jointures.</p>

    <h2>6. Interface utilisateur</h2>
    <h3>Page Taux de change</h3>
    <p>Accès via le groupe <strong>Consolidation</strong>. L'interface présente :</p>
    <ul>
      <li>Sélection du jeu de taux (<code>rate_set</code>) et de la période</li>
      <li>Grille des taux par devise source avec colonnes : taux_ouverture, taux_moyen, taux_close</li>
      <li>CRUD complet (création, modification, suppression)</li>
      <li>Import CSV possible via <code>POST /api/import/rates</code></li>
    </ul>

    <h2>7. API REST</h2>
    <h3>Routes CRUD</h3>
    <p>La table <code>rates</code> est exposée via le masterdata CRUD :</p>
    <ul>
      <li><code>GET /api/md/rates</code> — liste des taux</li>
      <li><code>POST /api/md/rates</code> — créer un taux</li>
      <li><code>PUT /api/md/rates/{`{id}`}</code> — modifier un taux</li>
      <li><code>DELETE /api/md/rates/{`{id}`}</code> — supprimer un taux</li>
    </ul>

    <h3>Import CSV</h3>
    <ul>
      <li><code>POST /api/import/rates</code> — upsert depuis un fichier CSV</li>
    </ul>

    <h2>8. Exemples concrets</h2>
    <h3>Exemple 1 : EUR/USD sur une période</h3>
    <p>Pour un jeu de taux <code>RATES</code> et une période <code>2024</code>, devise pivot = EUR :</p>
    <table className="help-table">
      <thead>
        <tr>
          <th>currency_source</th>
          <th>taux_ouverture</th>
          <th>taux_moyen</th>
          <th>taux_close</th>
        </tr>
      </thead>
      <tbody>
        <tr><td>USD</td><td>1.0800</td><td>1.0850</td><td>1.0900</td></tr>
      </tbody>
    </table>
    <p>Sémantique : <code>1 USD = taux × EUR</code> (ex. : 1.0900 USD = 1 EUR à la clôture).</p>

    <h3>Exemple 2 : Conversion d'un compte de bilan (USD → EUR)</h3>
    <p>Entité en USD, présentation en EUR, compte de bilan (schéma BILAN) :</p>
    <ul>
      <li><strong>F00 (ouverture)</strong> : 1000 USD × 1.0800 = 1080 EUR. Écart F80 = 1000 × (1.0900 − 1.0800) = 10 EUR.</li>
      <li><strong>F20 (variation)</strong> : 500 USD × 1.0850 = 542.50 EUR. Écart F81 = 500 × (1.0900 − 1.0850) = 2.50 EUR.</li>
      <li><strong>F99 (clôture)</strong> : 1500 USD × 1.0900 = 1635 EUR. Écart = 0.</li>
    </ul>
    <p>Vérification : F99_conv = F00_conv + F20_conv + F80 + F81 = 1080 + 542.50 + 10 + 2.50 = 1635 EUR ✓</p>

    <h3>Exemple 3 : Conversion d'un compte de résultat (USD → EUR)</h3>
    <p>Même entité, mais compte de résultat (schéma RESULTAT) :</p>
    <ul>
      <li><strong>F20 (variation)</strong> : 500 USD × 1.0850 = 542.50 EUR. <strong>Pas d'écart</strong> (flux_ecart = NULL).</li>
    </ul>

    <h3>Exemple 4 : Cross-rate pour présentation en GBP</h3>
    <p>Entité en USD, présentation en GBP, pivot = EUR. Taux vers pivot : USD = 1.0900, GBP = 0.8600.</p>
    <pre>
{`taux_cross(USD → GBP) = taux(USD → EUR) / taux(GBP → EUR) = 1.0900 / 0.8600 = 1.2674
1000 USD × 1.2674 = 1267.40 GBP`}
    </pre>

    <h3>Voir aussi</h3>
    <ul>
      <li><NavLink to='schemas-flux'>Schémas de flux (résolution du taux par compte)</NavLink></li>
      <li><NavLink to='taux-integration'>Pipeline de consolidation (étapes A-B-C-D)</NavLink></li>
    </ul>
  </div>
);

// Pages d'aide disponibles
const HELP_PAGES: HelpPage[] = [
  {
    id: 'postes-indicateurs',
    title: 'Postes et Indicateurs',
    content: postesIndicateursContent,
  },
  {
    id: 'coefficients',
    title: 'Coefficients',
    content: coefficientsContent,
  },
  {
    id: 'regles',
    title: 'Règles',
    content: reglesContent,
  },
  {
    id: 'schemas-flux',
    title: 'Schémas de flux',
    content: schemasFluxContent,
  },
  {
    id: 'taux-integration',
    title: "Taux d'intégration natif",
    content: tauxIntegrationContent,
  },
  {
    id: 'perimetres',
    title: 'Périmètres',
    content: perimetresContent,
  },
  {
    id: 'dimensions-masterdata',
    title: 'Dimensions et Master Data',
    content: dimensionsMasterdataContent,
  },
  {
    id: 'import-saisie',
    title: 'Import et Saisie',
    content: importSaisieContent,
  },
  {
    id: 'taux-change',
    title: 'Taux de change',
    content: tauxChangeContent,
  },
  {
    id: 'a-nouveau',
    title: 'À-nouveau',
    content: aNouveauContent,
  },
];

export function HelpPage() {
  const [activePage, setActivePage] = useState<string>(HELP_PAGES[0].id);

  const currentPage = HELP_PAGES.find((p) => p.id === activePage) ?? HELP_PAGES[0];

  return (
    <div className="page">
      <div className="page__header">
        <h2>Aide</h2>
        <p className="page__hint">Documentation de l'application</p>
      </div>

      <div className="help-layout">
        <nav className="help-sidebar">
          <h3>Sommaire</h3>
          <ul className="help-nav">
            {HELP_PAGES.map((page) => (
              <li key={page.id}>
                <button
                  type="button"
                  className={`help-nav-link ${activePage === page.id ? 'help-nav-link--active' : ''}`}
                  onClick={() => setActivePage(page.id)}
                >
                  {page.title}
                </button>
              </li>
            ))}
          </ul>
        </nav>

        <main className="help-main">
          <HelpNavContext.Provider value={setActivePage}>
            <div className="help-page">
              {currentPage.content}
            </div>
          </HelpNavContext.Provider>
        </main>
      </div>
    </div>
  );
}